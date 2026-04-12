#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // HTTP/1.1 request
    #[cfg(feature = "tls")]
    {
        let mut res = potato::get!("https://www.fawdlstty.com").await?;
        println!("HTTP/1.1 resp: {}", str::from_utf8(res.body.data().await)?);
    }
    
    // HTTP/2 request
    #[cfg(feature = "http2")]
    {
        let mut res = potato::get!(http2("https://www.fawdlstty.com")).await?;
        println!("HTTP/2 resp: {}", str::from_utf8(res.body.data().await)?);
    }
    
    // HTTP/3 request
    #[cfg(feature = "http3")]
    {
        let mut res = potato::get!(http3("https://www.fawdlstty.com")).await?;
        println!("HTTP/3 resp: {}", str::from_utf8(res.body.data().await)?);
    }
    
    Ok(())
}
