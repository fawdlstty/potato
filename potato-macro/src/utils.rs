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
            .replace("potato::HttpResponse", "HttpResponse");
        match ret.is_empty() {
            true => "()".to_string(),
            false => ret,
        }
    }
}
