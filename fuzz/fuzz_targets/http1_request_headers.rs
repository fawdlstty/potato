#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use potato::HttpRequest;

// HTTP/1.1 请求头解析 fuzz：打同步头解析入口 HttpRequest::from_headers_part
// （含 CL/TE/Host 歧义帧拒绝语义）与 server::buf_holds_complete_request
// flush 探测函数。内存安全由 libFuzzer + ASan 兜底，断言：
// * 接受的请求消耗量不越界，且该前缀可复现同一帧边界；
// * 原始扫描可见的歧义帧（冲突重复 CL、重复 TE、1.1 多 Host、非法 CL）
//   必被拒绝（对齐 hyper 走私防御语义）；
// * 对已接受的请求，探测函数返回值必须与按其文档语义（头解析完整、
//   无 TE/Expect、CL 全部可解析、头长+CL ≤ 缓冲长）复算的结果一致
//   ——探测与真解析漂移会造成流水线 flush 顺序错误。
fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 {
        // 超出默认 16KB 头上限，跳过以控制内存占用（限流分支由
        // http1_request_full 用小上限覆盖）
        return;
    }
    match HttpRequest::from_headers_part(data) {
        Ok(Some((_req, consumed))) => {
            assert!(consumed <= data.len(), "consumed 越界: {consumed} > {}", data.len());
            match HttpRequest::from_headers_part(&data[..consumed]) {
                Ok(Some((_, again))) => assert_eq!(again, consumed, "帧边界不稳定"),
                other => panic!("已接受的前缀复解析失败: {other:?}"),
            }
        }
        Ok(None) | Err(_) => {}
    }
    let facts = common::scan_request(data);
    if !facts.complete {
        return;
    }
    if let Ok(Some(_)) = HttpRequest::from_headers_part(data) {
        assert!(
            !facts.cl_conflicting(),
            "冲突重复 Content-Length 被接受（请求走私向量）: {:?}",
            facts.content_lengths
        );
        assert!(
            facts.transfer_encoding_headers.len() <= 1,
            "重复 Transfer-Encoding 被接受（请求走私向量）"
        );
        assert!(
            !(facts.host_count > 1 && facts.version_11),
            "HTTP/1.1 多 Host 被接受"
        );
        assert!(
            facts
                .content_lengths
                .iter()
                .all(|v| v.bytes().all(|b| b.is_ascii_digit())),
            "非法 Content-Length 被接受（请求走私向量）: {:?}",
            facts.content_lengths
        );
        if !facts.has_te() && !facts.expect_seen {
            // 按探测函数文档语义精确复算期望值（探测为 flush 决策依据，
            // 与真解析漂移会造成流水线顺序错误；仅断言已接受的请求）
            let expected = facts
                .content_lengths
                .iter()
                .all(|v| v.parse::<usize>().is_ok())
                && facts
                    .content_lengths
                    .first()
                    .and_then(|v| v.parse::<usize>().ok())
                    .map_or(true, |cl| facts.hdr_len.saturating_add(cl) <= data.len());
            assert_eq!(
                potato::server::buf_holds_complete_request(data),
                expected,
                "buf_holds_complete_request 与真解析漂移"
            );
        }
    }
});
