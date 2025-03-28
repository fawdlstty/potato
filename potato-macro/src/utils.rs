pub trait StringExt {
    fn type_simplify(&self) -> String;
}

impl StringExt for String {
    fn type_simplify(&self) -> String {
        let ret = self
            .replace("potato :: ", "")
            .replace("std :: ", "")
            .replace("net :: ", "")
            .replace("anyhow :: ", "")
            .replace("-> ", "");
        match ret.is_empty() {
            true => "()".to_string(),
            false => ret,
        }
    }
}
