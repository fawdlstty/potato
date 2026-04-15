/// 实测验证 max_concurrency 限制确实生效
///
/// 本示例启动服务器后，通过并发发送多个请求来验证：
/// 1. 并发请求数确实被限制在指定值
/// 2. 超过限制的请求会等待而不是被拒绝
use potato::{HttpResponse, HttpServer};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;

// 全局计数器：追踪当前并发数和峰值并发数
static CURRENT_CONCURRENT: AtomicU32 = AtomicU32::new(0);
static PEAK_CONCURRENT: AtomicU32 = AtomicU32::new(0);

// 限制为3个并发的handler
#[potato::http_get("/test-concurrency-3")]
#[potato::max_concurrency(3)]
async fn test_concurrency_3(delay_ms: u64) -> HttpResponse {
    // 增加当前并发计数
    let current = CURRENT_CONCURRENT.fetch_add(1, Ordering::SeqCst) + 1;

    // 更新峰值
    PEAK_CONCURRENT.fetch_max(current, Ordering::SeqCst);

    println!("[Handler] Request started, current concurrent: {}", current);

    // 模拟耗时操作
    sleep(Duration::from_millis(delay_ms)).await;

    // 减少当前并发计数
    CURRENT_CONCURRENT.fetch_sub(1, Ordering::SeqCst);

    println!("[Handler] Request finished");

    HttpResponse::text(format!("Completed with delay {}ms", delay_ms))
}

// 无限制的handler（用于对比）
#[potato::http_get("/test-no-limit")]
async fn test_no_limit(delay_ms: u64) -> HttpResponse {
    let current = CURRENT_CONCURRENT.fetch_add(1, Ordering::SeqCst) + 1;
    PEAK_CONCURRENT.fetch_max(current, Ordering::SeqCst);

    println!("[NoLimit] Request started, current concurrent: {}", current);

    sleep(Duration::from_millis(delay_ms)).await;

    CURRENT_CONCURRENT.fetch_sub(1, Ordering::SeqCst);

    println!("[NoLimit] Request finished");

    HttpResponse::text(format!("Completed with delay {}ms", delay_ms))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Max Concurrency 实测验证 ===\n");

    // 启动服务器
    let mut server = HttpServer::new("127.0.0.1:9090");
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.serve_http().await {
            eprintln!("Server error: {}", e);
        }
    });

    // 等待服务器启动
    sleep(Duration::from_millis(500)).await;

    println!("服务器已启动: http://127.0.0.1:9090\n");

    // 测试1：验证有限制的handler
    println!("=== 测试1: 限制为3个并发 ===");
    test_with_limit(3).await;

    // 重置计数器
    CURRENT_CONCURRENT.store(0, Ordering::SeqCst);
    PEAK_CONCURRENT.store(0, Ordering::SeqCst);

    sleep(Duration::from_secs(1)).await;

    // 测试2：验证无限制的handler（对比）
    println!("\n=== 测试2: 无限制（对比） ===");
    test_without_limit().await;

    println!("\n=== 测试完成 ===");
    println!("按 Ctrl+C 关闭服务器");

    // 等待服务器
    server_handle.await?;

    Ok(())
}

async fn test_with_limit(max_expected: u32) {
    let base_url = "http://127.0.0.1:9090";
    let num_requests = 10;
    let delay_per_request = 500; // 每个请求耗时500ms

    println!(
        "发送 {} 个并发请求，每个请求耗时 {}ms",
        num_requests, delay_per_request
    );
    println!("预期峰值并发数: {}\n", max_expected);

    let start_time = Instant::now();
    let mut handles = vec![];

    // 同时发送10个请求
    for i in 0..num_requests {
        let url = format!(
            "{}/test-concurrency-3?delay_ms={}",
            base_url, delay_per_request
        );
        let handle = tokio::spawn(async move {
            let req_start = Instant::now();
            match potato::get(&url, vec![]).await {
                Ok(res) => {
                    let elapsed = req_start.elapsed();
                    println!(
                        "[Client] Request {} completed in {:?}, status: {}",
                        i, elapsed, res.http_code
                    );
                }
                Err(e) => {
                    println!("[Client] Request {} failed: {}", i, e);
                }
            }
        });
        handles.push(handle);
    }

    // 等待所有请求完成
    for handle in handles {
        handle.await.unwrap();
    }

    let total_elapsed = start_time.elapsed();
    let peak = PEAK_CONCURRENT.load(Ordering::SeqCst);

    println!("\n结果统计:");
    println!("  总耗时: {:?}", total_elapsed);
    println!("  峰值并发数: {}", peak);
    println!("  预期最大并发: {}", max_expected);

    if peak <= max_expected {
        println!("  ✅ 验证通过: 并发数被正确限制在 {} 以内", max_expected);
    } else {
        println!("  ❌ 验证失败: 并发数 {} 超过了限制 {}", peak, max_expected);
    }

    // 理论耗时计算
    // 如果限制为3，10个请求需要分4批：3+3+3+1
    // 每批500ms，总耗时应约为 4 * 500 = 2000ms
    let expected_batches = (num_requests as f64 / max_expected as f64).ceil() as u64;
    let expected_time = Duration::from_millis(expected_batches * delay_per_request);

    println!("  理论分批数: {}", expected_batches);
    println!("  理论最小耗时: {:?}", expected_time);

    if total_elapsed >= expected_time * 9 / 10 {
        println!("  ✅ 验证通过: 总耗时符合预期（请求被分批执行）");
    } else {
        println!("  ⚠️  警告: 总耗时低于预期，可能限制未生效");
    }
}

async fn test_without_limit() {
    let base_url = "http://127.0.0.1:9090";
    let num_requests = 10;
    let delay_per_request = 500;

    println!(
        "发送 {} 个并发请求，每个请求耗时 {}ms",
        num_requests, delay_per_request
    );
    println!("预期峰值并发数: {}（无限制）\n", num_requests);

    let start_time = Instant::now();
    let mut handles = vec![];

    for i in 0..num_requests {
        let url = format!("{}/test-no-limit?delay_ms={}", base_url, delay_per_request);
        let handle = tokio::spawn(async move {
            let req_start = Instant::now();
            match potato::get(&url, vec![]).await {
                Ok(res) => {
                    let elapsed = req_start.elapsed();
                    println!(
                        "[Client] Request {} completed in {:?}, status: {}",
                        i, elapsed, res.http_code
                    );
                }
                Err(e) => {
                    println!("[Client] Request {} failed: {}", i, e);
                }
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let total_elapsed = start_time.elapsed();
    let peak = PEAK_CONCURRENT.load(Ordering::SeqCst);

    println!("\n结果统计:");
    println!("  总耗时: {:?}", total_elapsed);
    println!("  峰值并发数: {}", peak);
    println!("  预期并发数: {}", num_requests);

    if peak == num_requests {
        println!("  ✅ 验证通过: 无限制时所有请求并发执行");
    } else {
        println!("  ⚠️  注意: 峰值并发 {} 小于请求数 {}", peak, num_requests);
    }

    // 无限制时，所有请求应该并发执行，总耗时应接近500ms
    if total_elapsed < Duration::from_millis(delay_per_request * 2) {
        println!("  ✅ 验证通过: 总耗时接近单次请求耗时（并发执行）");
    } else {
        println!("  ⚠️  警告: 总耗时过长");
    }
}
