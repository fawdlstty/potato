# ACME自动证书管理

potato框架内置了ACME协议支持，可以自动从Let's Encrypt等证书颁发机构获取和续期TLS证书，无需手动管理证书文件。

## 功能特性

- **全自动证书管理**：首次启动自动申请证书
- **自动续期**：后台自动检测并续期即将过期的证书
- **热重载**：证书更新后自动应用，无需重启服务器
- **HTTP-01验证**：自动处理验证挑战，无需手动配置

## 快速开始

### 启用ACME功能

在`Cargo.toml`中启用`acme`特性：

```toml
[dependencies]
potato = { version = "0.3", features = ["acme"] }
```

### 最简示例

只需提供域名和邮箱，其他一切自动处理：

```rust
#[potato::http_get("/hello")]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("Hello from ACME TLS!")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:443");
    
    // 一行代码启用ACME自动证书
    server.serve_acme("example.com", "admin@example.com").await
}
```

就是这么简单！服务器会：
1. 自动注册ACME账户
2. 自动申请Let's Encrypt证书
3. 自动处理HTTP-01验证挑战
4. 自动续期证书（每6小时检查一次）

## 高级配置

如果需要更多控制，可以使用`serve_acme_with_opts`方法：

```rust
use potato::acme::AcmeOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:443");
    
    let opts = AcmeOptions {
        // 支持多域名（SAN证书）
        domains: vec![
            "example.com".to_string(),
            "www.example.com".to_string(),
        ],
        email: "admin@example.com".to_string(),
        // 自定义ACME目录URL（可选）
        // 默认使用Let's Encrypt生产环境
        acme_directory: None,
        // 证书缓存目录（可选）
        // 默认 "./acme_certs"
        cert_dir: Some("./my_certs".to_string()),
    };
    
    server.serve_acme_with_opts(opts).await
}
```

## 工作原理

### 首次启动流程

1. **账户注册**：在`acme_certs/`目录创建ACME账户（使用ECDSA P-256密钥）
2. **证书申请**：向Let's Encrypt申请证书
3. **HTTP-01验证**：
   - Let's Encrypt访问 `http://your-domain.com/.well-known/acme-challenge/{token}`
   - potato自动响应正确的验证令牌
4. **证书保存**：将证书和私钥保存到`acme_certs/cert.pem`和`acme_certs/key.pem`
5. **HTTPS服务**：使用获得的证书启动HTTPS服务

### 自动续期流程

后台任务每6小时检查一次证书状态：
- 如果证书创建超过60天（Let's Encrypt证书有效期90天），自动续期
- 续期成功后自动重载TLS配置，无需重启
- 续期失败不影响当前证书，继续尝试

## 证书存储

ACME模块会在指定目录（默认`./acme_certs`）保存以下文件：

```
acme_certs/
├── account.json    # ACME账户凭据（用于避免重复注册）
├── cert.pem        # TLS证书链
└── key.pem         # 私钥
```

**重要**：请确保此目录的安全性，特别是`key.pem`文件包含敏感信息。

## 生产环境部署

### 前提条件

1. **域名DNS配置**：域名必须已解析到服务器IP
2. **端口可访问**：80和443端口必须对公网可访问
3. **防火墙配置**：确保Let's Encrypt可以访问你的服务器

### 使用系统服务

创建systemd服务文件 `/etc/systemd/system/potato-acme.service`：

```ini
[Unit]
Description=Potato ACME HTTP Server
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/var/www/your-app
ExecStart=/var/www/your-app/potato-acme-server
Restart=on-failure
RestartSec=5

# 确保ACME证书目录可写
ReadWritePaths=/var/www/your-app/acme_certs

[Install]
WantedBy=multi-user.target
```

### 首次部署注意事项

首次启动时，证书申请可能需要一些时间（通常30秒-2分钟）：

```
Starting ACME server on port 443
Certificate will be automatically provisioned for your domain

[ACME] Requesting certificate for example.com...
[ACME] Validating HTTP-01 challenge...
[ACME] Certificate issued successfully
[ACME] Starting HTTPS server...
```

## 与serve_https的区别

| 特性 | `serve_https` | `serve_acme` |
|------|---------------|--------------|
| 证书来源 | 手动提供文件 | 自动从CA申请 |
| 证书续期 | 需手动更新文件 | 自动续期 |
| 重启需求 | 更新证书需重启 | 热重载，无需重启 |
| 适用场景 | 内网、测试、自定义CA | 生产环境、公网服务 |
| 配置复杂度 | 需自行管理证书 | 零证书管理 |

### 何时使用serve_https

- 使用自签名证书（内网环境）
- 使用企业CA签发的证书
- 需要客户端证书认证
- 测试环境快速启动

### 何时使用serve_acme

- 生产环境公网服务
- 需要Let's Encrypt免费证书
- 不想手动管理证书生命周期
- 需要自动续期避免服务中断

## 故障排查

### 证书申请失败

**问题**：启动时证书申请失败

**可能原因**：
1. 域名未正确解析到服务器IP
2. 80端口被防火墙阻止
3. 其他服务占用了80端口

**解决方法**：
```bash
# 检查域名解析
dig example.com

# 检查80端口是否可访问
curl http://example.com/.well-known/acme-challenge/test

# 确保80和443端口未被占用
sudo lsof -i :80
sudo lsof -i :443
```

### 证书续期失败

**问题**：运行中证书续期失败

**查看日志**：
```
[ACME] Certificate expiring soon, renewing...
[ACME] Failed to renew certificate: ...
```

**解决方法**：
1. 检查网络连接
2. 检查Let's Encrypt服务状态：https://letsencrypt.status.io/
3. 删除`acme_certs/`目录重新开始（会重新注册账户）

### 切换ACME环境

默认使用Let's Encrypt生产环境。如需测试，可使用staging环境：

```rust
let opts = AcmeOptions {
    domains: vec!["example.com".to_string()],
    email: "admin@example.com".to_string(),
    // Let's Encrypt Staging环境（有速率限制但不会耗尽生产配额）
    acme_directory: Some("https://acme-staging-v02.api.letsencrypt.org/directory".to_string()),
    cert_dir: None,
};
```

**注意**：Staging环境签发的证书不被浏览器信任，仅用于测试。

## 安全建议

1. **保护私钥**：确保证书目录权限设置正确
   ```bash
   chmod 700 acme_certs/
   chmod 600 acme_certs/key.pem
   ```

2. **定期备份**：备份`acme_certs/`目录，特别是`account.json`

3. **监控续期**：虽然自动续期，但仍建议监控日志确保续期成功

4. **使用生产环境**：测试完成后务必切换到生产环境URL
