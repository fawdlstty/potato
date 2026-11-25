#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use potato::utils::tcp_stream::HttpStream;
use potato::HttpRequest;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

// 输入上限：与 duplex 缓冲（64KB）配合，保证 write_all 永不阻塞。
const MAX_INPUT: usize = 32 * 1024;
const DUPLEX_BUF: usize = 64 * 1024;

// HTTP/1.1 请求完整分帧 fuzz：头解析 + CL/chunked body 状态机
// （HttpRequest::from_stream_with_limits，经 EOF duplex 流喂入）。
// 输入按首字节决定预灌缓冲量，覆盖"半包到达"重组装路径。
// 内存安全由 libFuzzer + ASan 兜底，断言：
// * chunked 尺寸行/扩展/终止符/trailer 状态机对任意输入不 panic；
// * 接受的请求消耗量不越界；
// * 分帧层歧义帧拒绝语义：TE+CL 并存、非纯 chunked TE、重复 TE、
//   冲突重复 CL 一律不被接受（对齐 hyper 走私防御语义）；
// * 小上限（16KB 头/64KB 体）下限流拒绝分支正常工作。
fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > MAX_INPUT {
        return;
    }
    let facts = common::scan_request(data);
    // 头部阶段先行：头解析单独接受但分帧层拒绝的路径也要走到
    let headers_ok = matches!(HttpRequest::from_headers_part(data), Ok(Some(_)));
    let split = (data[0] as usize % 17).min(data.len());
    let (result, buf_len) = potato::block_on(async {
        let (mut client, server) = tokio::io::duplex(DUPLEX_BUF);
        let stream = HttpStream::from_duplex_stream(server);
        client
            .write_all(&data[split..])
            .await
            .expect("duplex 写入不应失败（读端存活且缓冲足够）");
        // 写完即 drop 产生 EOF：所有读路径在数据耗尽后必然报错返回
        drop(client);
        let mut buf = data[..split].to_vec();
        let stream = Arc::new(Mutex::new(stream));
        let result = HttpRequest::from_stream_with_limits(
            &mut buf,
            stream,
            Some(16 * 1024),
            Some(64 * 1024),
        )
        .await;
        (result, buf.len())
    });
    if let Ok((_req, consumed)) = result {
        assert!(consumed <= buf_len, "consumed 越界: {consumed} > {buf_len}");
        if headers_ok && facts.complete {
            assert!(
                !(facts.has_te() && facts.has_cl()),
                "TE+CL 并存被完整分帧接受（请求走私向量）"
            );
            if facts.has_te() {
                assert!(facts.te_plain_chunked(), "非纯 chunked TE 被接受");
            }
            assert!(
                facts.transfer_encoding_headers.len() <= 1,
                "重复 Transfer-Encoding 被接受（请求走私向量）"
            );
            assert!(
                !facts.cl_conflicting(),
                "冲突重复 Content-Length 被接受（请求走私向量）: {:?}",
                facts.content_lengths
            );
            assert!(
                facts
                    .content_lengths
                    .iter()
                    .all(|v| v.bytes().all(|b| b.is_ascii_digit()) && v.parse::<usize>().is_ok()),
                "非法 Content-Length 被完整分帧接受（请求走私向量）: {:?}",
                facts.content_lengths
            );
        }
    }
});
