# 前言

轮子是永恒的话题。说到新项目，不得不将为什么又开一个新的坑。首先讲讲我对Rust语言的观感，语言本身设计挺不错的，但库的API可读性太差，仿佛每个写库的人都不知道什么叫“清晰”、“直观”、“好用”。延伸到HTTP领域，相信大家看过下面的基础示例，会有感觉。

首先是axum的hello world:

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

然后是actix web的hello world：

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

再之后是ntex的hello world：

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

不知为什么，即使是hello world示例，也都设计的非常绕，语法也极不简练，最大的问题是每个处理函数都得手工去注册，这点所有HTTP框架都做到了统一，这也是我最不理解的点。

除此以外就是HTTP的客户端库。参考了一下Rust语言的reqwest库，除了最简单的`reqwest::get`外，其他用法都极为繁杂。

某个库难用固然和设计思想等因素相关，但每个库都做到如此难用，着实出乎我的意料。我希望改变一下现状，开发一个新的HTTP框架，包含客户端与服务器端，主要思想是，对开发者提供极简的API接口，能简则简，这就是potato项目的由来
