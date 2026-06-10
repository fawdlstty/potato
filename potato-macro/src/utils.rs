pub trait StringExt {
    fn type_simplify(&self) -> String;
}

impl StringExt for String {
    fn type_simplify(&self) -> String {
        let ret = self
            .replace(" :: ", "::")
            .replace(" <", "<")
            .replace("< ", "<")
            .replace(" >", ">")
            .replace("> ", ">")
            .replace("-> ", "")
            .replace("->", "")
            .replace("anyhow::Result", "Result")
            .replace("potato::HttpRequest", "HttpRequest")
            .replace("potato::HttpResponse", "HttpResponse")
            .replace("potato::SessionCache", "SessionCache")
            .replace("potato::OnceCache", "OnceCache")
            .replace("potato_lite::HttpRequest", "HttpRequest")
            .replace("potato_lite::HttpResponse", "HttpResponse")
            .replace("potato_lite::SessionCache", "SessionCache")
            .replace("potato_lite::OnceCache", "OnceCache");
        match ret.is_empty() {
            true => "()".to_string(),
            false => ret,
        }
    }
}
