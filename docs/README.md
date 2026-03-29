---
home: true
title: Home
#heroImage: https://vuejs.press/images/hero.png
actions:
  - text: 入门示例
    link: /guide/01_hello_world.md
    type: primary
  - text: 查看源码
    link: https://github.com/fawdlstty/potato
    type: secondary

features:
  - title: 高性能、极简
    details: 纯Rust语言实现，速度快，几行代码即可实现HTTP服务器/客户端
  - title: 多协议服务端
    details: 同一套处理函数可运行在 HTTP/1.1、HTTP/2、HTTP/3
  - title: OpenAPI支持
    details: 支持OpenAPI规范，支持生成API文档
  - title: 静态资源打包
    details: 支持静态HTTP资源文件打包，方便部署
  - title: 条件请求与分段下载
    details: 静态文件路由支持 ETag/Last-Modified 与 Range/If-Range
  - title: JwtAuth校验支持
    details: 内置JwtAuth校验功能，方便API鉴权
  - title: 内存泄露排查
    details: 内置jemalloc内存泄露检测功能，方便排查内存泄露
  - title: WebDAV 支持
    details: 支持服务器端 WebDAV 协议
  - title: AI 协议支持
    details: 支持 OpenAI 和 Claude 两种主流 AI 流式传输协议

footer: CC-BY 4.0 Licensed | Copyright © 2025 potato
---

<!--
npm run docs:build
npm run docs:dev
-->
