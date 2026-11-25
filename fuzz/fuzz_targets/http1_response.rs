#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use potato::utils::tcp_stream::HttpStream;
use potato::{HttpMethod, HttpResponse};
use tokio::io::AsyncWriteExt;

// 输入上限：与 duplex 缓冲（64KB）配合，保证 write_all 永不阻塞；
// 也把 CL/chunked 大体的读循环开销控制在有限范围。
const MAX_INPUT: usize = 32 * 1024;
const DUPLEX_BUF: usize = 64 * 1024;

// HTTP/1.1 响应解析 fuzz：头解析 + CL/chunked body 状态机（客户端侧，
// 面对敌意服务器输入）。首字节选择请求方法（None/GET/HEAD），覆盖
// HEAD/1xx/204/304 无 body 分支与 100 interim 丢弃循环。
// 内存安全由 libFuzzer + ASan 兜底，断言：
// * chunked 尺寸行/终止符/trailer 状态机对任意输入不 panic；
// * 接受的响应消耗量不越界；
// * 歧义帧拒绝语义：TE+CL 并存、重复 TE、冲突重复 CL、chunked 非最终
//   编码一律不被接受（对齐 hyper 走私防御语义）。
fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > MAX_INPUT {
        return;
    }
    let method = match data[0] % 3 {
        0 => None,
        1 => Some(HttpMethod::GET),
        _ => Some(HttpMethod::HEAD),
    };
    let facts = common::scan_response(data);
    let headers_ok = matches!(HttpResponse::from_headers_part(data), Ok(Some(_)));
    let split = (data[0] as usize >> 2) % 17.min(data.len());
    let (result, buf_len) = potato::block_on(async {
        let (mut client, server) = tokio::io::duplex(DUPLEX_BUF);
        let mut stream = HttpStream::from_duplex_stream(server);
        client
            .write_all(&data[split..])
            .await
            .expect("duplex 写入不应失败（读端存活且缓冲足够）");
        // 写完即 drop 产生 EOF：所有读路径在数据耗尽后必然报错返回
        drop(client);
        let mut buf = data[..split].to_vec();
        let result =
            HttpResponse::from_stream_with_request_method(&mut buf, &mut stream, method).await;
        (result, buf.len())
    });
    if let Ok((_res, consumed)) = result {
        assert!(consumed <= buf_len, "consumed 越界: {consumed} > {buf_len}");
        if headers_ok && facts.complete {
            assert!(
                !(facts.has_te() && facts.has_cl()),
                "TE+CL 并存被接受（走私向量）"
            );
            // HEAD/1xx/204/304 无 body 响应在分帧前提前返回，TE 不参与
            // 分帧判定，chunked-final 语义只约束有 body 的响应
            let skip_body = method == Some(HttpMethod::HEAD)
                || (100..200).contains(&facts.status)
                || facts.status == 204
                || facts.status == 304;
            if facts.has_te() && !skip_body {
                assert!(facts.te_chunked_final(), "chunked 非最终编码的 TE 被接受");
            }
            assert!(
                facts.transfer_encoding_headers.len() <= 1,
                "重复 Transfer-Encoding 被接受（走私向量）"
            );
            assert!(
                !facts.cl_conflicting(),
                "冲突重复 Content-Length 被接受（走私向量）: {:?}",
                facts.content_lengths
            );
            assert!(
                facts
                    .content_lengths
                    .iter()
                    .all(|v| v.bytes().all(|b| b.is_ascii_digit()) && v.parse::<usize>().is_ok()),
                "非法 Content-Length 被接受（走私向量）: {:?}",
                facts.content_lengths
            );
        }
    }
});
