pub trait StringExt {
    fn type_simplify(&self) -> String;
    fn http_std_case(&self) -> String;
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

    fn http_std_case(&self) -> String {
        let mut ret = "".to_string();
        let mut upper = true;
        for ch in self.chars() {
            if ch == '-' {
                upper = true;
                ret.push(ch);
            } else if upper {
                ret.push(ch.to_ascii_uppercase());
                upper = false;
            } else {
                ret.push(ch.to_ascii_lowercase());
            }
        }
        ret
    }
}
