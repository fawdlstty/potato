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

pub trait StrExt {
    fn url_decode(&self) -> String;
}

impl StrExt for &str {
    fn url_decode(&self) -> String {
        let mut ret = vec![];
        let mut chars = self.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '%' => {
                    let hex = chars.next().unwrap_or('0').to_digit(16).unwrap_or(0) << 4
                        | chars.next().unwrap_or('0').to_digit(16).unwrap_or(0);
                    ret.push(hex as u8);
                }
                '+' => {
                    ret.push(b' ');
                }
                _ => {
                    ret.push(ch as u8);
                }
            }
        }
        String::from_utf8(ret).unwrap_or("".to_string())
    }
}
