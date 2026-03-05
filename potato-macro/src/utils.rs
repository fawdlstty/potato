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
            .replace("tokio::sync::mpsc::Receiver", "Receiver")
            .replace("tokio::sync::mpsc::Sender", "Sender")
            .replace("sync::mpsc::Receiver", "Receiver")
            .replace("sync::mpsc::Sender", "Sender")
            .replace("mpsc::Receiver", "Receiver")
            .replace("mpsc::Sender", "Sender")
            .replace("Vec<u8>", "Vec<u8>")
            .replace("Vec", "Vec")
            .replace("u8", "u8")
            .replace("Receiver", "Receiver")
            .replace("Sender", "Sender");
        match ret.is_empty() {
            true => "()".to_string(),
            false => ret,
        }
    }
}
