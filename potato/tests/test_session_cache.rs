/// SessionCache 功能测试
use potato::SessionCache;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_session_cache_token_generation() -> anyhow::Result<()> {
    // 测试 token 签发和解析
    SessionCache::set_jwt_secret(b"test-secret-key").await;

    let token = SessionCache::generate_token(12345, Duration::from_secs(3600)).await?;
    assert!(!token.is_empty());

    let (user_id, ttl) = SessionCache::parse_token(&token)
        .await
        .expect("Token should be valid");
    assert_eq!(user_id, 12345);
    assert!(ttl.as_secs() > 3500); // 应该有接近3600秒（加1分钟缓冲）

    println!("✅ Token generation and parsing test passed");
    Ok(())
}

#[tokio::test]
async fn test_session_cache_from_token() -> anyhow::Result<()> {
    SessionCache::set_jwt_secret(b"test-secret-key").await;

    let token = SessionCache::generate_token(99999, Duration::from_secs(3600)).await?;
    let cache = SessionCache::from_token(&token)
        .await
        .expect("Should create session cache");

    // 设置一些数据
    cache.set("user_name", "test_user".to_string());
    cache.set("login_count", 5u32);

    // 获取数据
    let user_name: String = cache.get("user_name").expect("user_name not found");
    let login_count: u32 = cache.get("login_count").expect("login_count not found");

    assert_eq!(user_name, "test_user");
    assert_eq!(login_count, 5);

    println!("✅ Session cache from token test passed");
    Ok(())
}

#[tokio::test]
async fn test_session_cache_cross_request() -> anyhow::Result<()> {
    SessionCache::set_jwt_secret(b"test-secret-key").await;

    // 生成token
    let token = SessionCache::generate_token(77777, Duration::from_secs(3600)).await?;

    // 直接测试 SessionCache 的跨请求能力
    let cache1 = SessionCache::from_token(&token)
        .await
        .expect("Should create cache");
    cache1.set("request_count", 1u32);

    let cache2 = SessionCache::from_token(&token)
        .await
        .expect("Should get same cache");
    let count: u32 = cache2
        .get("request_count")
        .expect("request_count not found");
    assert_eq!(count, 1);

    cache2.set("request_count", 2u32);

    let cache3 = SessionCache::from_token(&token)
        .await
        .expect("Should get same cache");
    let count: u32 = cache3
        .get("request_count")
        .expect("request_count not found");
    assert_eq!(count, 2);

    println!("✅ Cross-request session cache test passed");
    Ok(())
}

#[tokio::test]
async fn test_session_cache_expiration() -> anyhow::Result<()> {
    SessionCache::set_jwt_secret(b"test-secret-key").await;

    // 生成一个只有1秒有效期的token
    let token = SessionCache::generate_token(88888, Duration::from_millis(800)).await?;

    // 首次应该成功
    let cache1 = SessionCache::from_token(&token).await;
    assert!(cache1.is_ok());
    cache1.unwrap().set("data", "test".to_string());

    // 等待过期
    sleep(Duration::from_millis(1200)).await;

    // 过期后应该失败（token和session都过期了）
    let cache2 = SessionCache::from_token(&token).await;
    assert!(cache2.is_err());

    println!("✅ Session cache expiration test passed");
    Ok(())
}

#[tokio::test]
async fn test_session_cache_cleanup() -> anyhow::Result<()> {
    SessionCache::set_jwt_secret(b"test-secret-key").await;

    // 创建几个session
    let token1 = SessionCache::generate_token(11111, Duration::from_millis(500)).await?; // 0.5秒过期
    let token2 = SessionCache::generate_token(22222, Duration::from_secs(3600)).await?; // 1小时过期

    let _ = SessionCache::from_token(&token1).await;
    let _ = SessionCache::from_token(&token2).await;

    // 等待第一个session过期
    sleep(Duration::from_millis(1000)).await;

    // 第一个token已过期，parse_token应该返回Error
    let parsed1 = SessionCache::parse_token(&token1).await;
    assert!(parsed1.is_err());

    // 第二个token仍然有效
    let parsed2 = SessionCache::parse_token(&token2).await;
    assert!(parsed2.is_ok());

    // 验证过期token无法获取cache
    let cache1 = SessionCache::from_token(&token1).await;
    assert!(cache1.is_err());

    let cache2 = SessionCache::from_token(&token2).await;
    assert!(cache2.is_ok());

    println!("✅ Session cache cleanup test passed");
    Ok(())
}

#[tokio::test]
async fn test_session_cache_different_users() -> anyhow::Result<()> {
    SessionCache::set_jwt_secret(b"test-secret-key").await;

    // 为不同用户生成token
    let token1 = SessionCache::generate_token(10001, Duration::from_secs(3600)).await?;
    let token2 = SessionCache::generate_token(10002, Duration::from_secs(3600)).await?;

    let cache1 = SessionCache::from_token(&token1)
        .await
        .expect("User 1 cache");
    let cache2 = SessionCache::from_token(&token2)
        .await
        .expect("User 2 cache");

    // 设置不同的数据
    cache1.set("user_id", 10001u64);
    cache2.set("user_id", 10002u64);

    // 验证数据隔离
    let user1_id: u64 = cache1.get("user_id").expect("user1_id not found");
    let user2_id: u64 = cache2.get("user_id").expect("user2_id not found");

    assert_eq!(user1_id, 10001);
    assert_eq!(user2_id, 10002);

    println!("✅ Different users session isolation test passed");
    Ok(())
}
