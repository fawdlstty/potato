# ACME Automatic Certificate Management

The potato framework has built-in ACME protocol support, allowing automatic acquisition and renewal of TLS certificates from Let's Encrypt and other Certificate Authorities without manual certificate management.

## Features

- **Fully Automatic Certificate Management**: Automatically request certificates on first startup
- **Automatic Renewal**: Background tasks automatically detect and renew expiring certificates
- **Hot Reload**: Automatically apply certificate updates without server restart
- **HTTP-01 Validation**: Automatically handle validation challenges without manual configuration

## Quick Start

### Enable ACME Feature

Enable the `acme` feature in `Cargo.toml`:

```toml
[dependencies]
potato = { version = "0.3", features = ["acme"] }
```

### Minimal Example

Just provide your domain and email, everything else is handled automatically:

```rust
#[potato::http_get("/hello")]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("Hello from ACME TLS!")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:443");
    
    // One line to enable ACME automatic certificates
    server.serve_acme("example.com", "admin@example.com").await
}
```

It's that simple! The server will:
1. Automatically register an ACME account
2. Automatically request a Let's Encrypt certificate
3. Automatically handle HTTP-01 validation challenges
4. Automatically renew certificates (checks every 6 hours)

## Advanced Configuration

For more control, use the `serve_acme_with_opts` method:

```rust
use potato::acme::AcmeOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:443");
    
    let opts = AcmeOptions {
        // Support multiple domains (SAN certificate)
        domains: vec![
            "example.com".to_string(),
            "www.example.com".to_string(),
        ],
        email: "admin@example.com".to_string(),
        // Custom ACME directory URL (optional)
        // Defaults to Let's Encrypt production
        acme_directory: None,
        // Certificate cache directory (optional)
        // Defaults to "./acme_certs"
        cert_dir: Some("./my_certs".to_string()),
    };
    
    server.serve_acme_with_opts(opts).await
}
```

## How It Works

### First Startup Flow

1. **Account Registration**: Create ACME account in `acme_certs/` directory (using ECDSA P-256 key)
2. **Certificate Request**: Request certificate from Let's Encrypt
3. **HTTP-01 Validation**:
   - Let's Encrypt accesses `http://your-domain.com/.well-known/acme-challenge/{token}`
   - potato automatically responds with the correct validation token
4. **Certificate Save**: Save certificate and private key to `acme_certs/cert.pem` and `acme_certs/key.pem`
5. **HTTPS Service**: Start HTTPS service with the obtained certificate

### Automatic Renewal Flow

Background task checks certificate status every 6 hours:
- If certificate is older than 60 days (Let's Encrypt certificates are valid for 90 days), automatically renew
- Automatically reload TLS configuration after successful renewal, no restart needed
- Renewal failures don't affect the current certificate, will continue trying

## Certificate Storage

The ACME module saves the following files in the specified directory (default `./acme_certs`):

```
acme_certs/
├── account.json    # ACME account credentials (to avoid re-registration)
├── cert.pem        # TLS certificate chain
└── key.pem         # Private key
```

**Important**: Ensure the security of this directory, especially the `key.pem` file which contains sensitive information.

## Production Deployment

### Prerequisites

1. **Domain DNS Configuration**: Domain must resolve to server IP
2. **Port Accessibility**: Ports 80 and 443 must be publicly accessible
3. **Firewall Configuration**: Ensure Let's Encrypt can access your server

### Using System Service

Create systemd service file `/etc/systemd/system/potato-acme.service`:

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

# Ensure ACME certificate directory is writable
ReadWritePaths=/var/www/your-app/acme_certs

[Install]
WantedBy=multi-user.target
```

### First Deployment Notes

On first startup, certificate issuance may take some time (typically 30 seconds to 2 minutes):

```
Starting ACME server on port 443
Certificate will be automatically provisioned for your domain

[ACME] Requesting certificate for example.com...
[ACME] Validating HTTP-01 challenge...
[ACME] Certificate issued successfully
[ACME] Starting HTTPS server...
```

## Difference from serve_https

| Feature | `serve_https` | `serve_acme` |
|---------|---------------|--------------|
| Certificate Source | Manual file provision | Automatic from CA |
| Certificate Renewal | Manual file update required | Automatic renewal |
| Restart Required | Restart needed for certificate updates | Hot reload, no restart |
| Use Case | Intranet, testing, custom CA | Production, public services |
| Configuration Complexity | Manual certificate management | Zero certificate management |

### When to Use serve_https

- Using self-signed certificates (intranet)
- Using enterprise CA-issued certificates
- Need client certificate authentication
- Quick setup for testing environments

### When to Use serve_acme

- Production public services
- Need free Let's Encrypt certificates
- Don't want to manually manage certificate lifecycle
- Need automatic renewal to avoid service interruption

## Troubleshooting

### Certificate Request Failed

**Problem**: Certificate request fails on startup

**Possible Causes**:
1. Domain not correctly resolving to server IP
2. Port 80 blocked by firewall
3. Another service occupying port 80

**Solutions**:
```bash
# Check domain resolution
dig example.com

# Check if port 80 is accessible
curl http://example.com/.well-known/acme-challenge/test

# Ensure ports 80 and 443 are not occupied
sudo lsof -i :80
sudo lsof -i :443
```

### Certificate Renewal Failed

**Problem**: Certificate renewal fails during runtime

**Check Logs**:
```
[ACME] Certificate expiring soon, renewing...
[ACME] Failed to renew certificate: ...
```

**Solutions**:
1. Check network connectivity
2. Check Let's Encrypt service status: https://letsencrypt.status.io/
3. Delete `acme_certs/` directory and start fresh (will re-register account)

### Switching ACME Environments

By default, uses Let's Encrypt production environment. For testing, you can use staging:

```rust
let opts = AcmeOptions {
    domains: vec!["example.com".to_string()],
    email: "admin@example.com".to_string(),
    // Let's Encrypt Staging environment (has rate limits but won't exhaust production quota)
    acme_directory: Some("https://acme-staging-v02.api.letsencrypt.org/directory".to_string()),
    cert_dir: None,
};
```

**Note**: Certificates issued by the staging environment are not trusted by browsers, for testing only.

## Security Recommendations

1. **Protect Private Keys**: Ensure correct permissions on certificate directory
   ```bash
   chmod 700 acme_certs/
   chmod 600 acme_certs/key.pem
   ```

2. **Regular Backups**: Backup `acme_certs/` directory, especially `account.json`

3. **Monitor Renewal**: Although automatic, it's recommended to monitor logs to ensure renewal success

4. **Use Production Environment**: After testing, be sure to switch to production environment URL
