pub trait StringExt {
    fn http_standardization(&self) -> String;
}

impl StringExt for &str {
    fn http_standardization(&self) -> String {
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

impl StringExt for String {
    fn http_standardization(&self) -> String {
        (&self[..]).http_standardization()
    }
}
