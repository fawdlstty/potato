# Preface

Wheels are an eternal topic. Speaking of new projects, we must address why another new project is being started. First, let me share my impressions of the Rust language. The language itself is well-designed, but the API readability of libraries is too poor. It seems like everyone who writes libraries doesn't know what "clarity," "intuitiveness," or "usability" mean. Extending to the HTTP domain, I believe those who see the following basic examples will have a feeling.

First is axum's hello world:

```rust
use axum::{response::Html, routing::get, Router};

#[tokio::main]
async fn main() {
    // build our application with a route
    let app = Router::new().route("/", get(handler));

    // run it
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn handler() -> Html<&'static str> {
    Html("<h1>Hello, World!</h1>")
}
```

Then actix web's hello world:

```rust
use actix_web::{App, HttpRequest, HttpServer, middleware, web};

async fn index(req: HttpRequest) -> &'static str {
    println!("REQ: {req:?}");
    "Hello world!"
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(web::resource("/").to(index))
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
```

After that is ntex's hello world:

```rust
use ntex::web;

#[web::get("/")]
async fn hello() -> impl web::Responder {
    web::HttpResponse::Ok().body("Hello world!")
}

#[ntex::main]
async fn main() -> std::io::Result<()> {
    web::HttpServer::new(|| {
        web::App::new().service(hello)
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
```

For some reason, even hello world examples are designed to be very convoluted, with syntax that is extremely verbose. The biggest problem is that each handler function must be manually registered. This is a point that all HTTP frameworks have unified on, and it's also the most incomprehensible point for me.

In addition to this, there are HTTP client libraries. After reviewing Rust's reqwest library, apart from the simplest `reqwest::get`, other usage patterns are extremely complex.

While a library being difficult to use is certainly related to design philosophy and other factors, when every library is as difficult to use as this, it's truly surprising. I hope to change the current situation by developing a new HTTP framework that includes both client and server sides. The main idea is to provide developers with extremely simple API interfaces, simplifying wherever possible. This is the origin of the potato project.