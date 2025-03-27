use rand::Rng;
use std::sync::LazyLock;

pub trait StringExt {
    fn http_std_case(&self) -> String;
    fn url_decode(&self) -> String;
    fn starts_with_ignore_ascii_case(&self, other: &str) -> bool;
}

impl StringExt for str {
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

    fn starts_with_ignore_ascii_case(&self, other: &str) -> bool {
        if self.len() < other.len() {
            return false;
        }
        (&self[..other.len()]).eq_ignore_ascii_case(other)
    }
}

impl StringExt for String {
    fn http_std_case(&self) -> String {
        self[..].http_std_case()
    }

    fn url_decode(&self) -> String {
        self[..].url_decode()
    }

    fn starts_with_ignore_ascii_case(&self, other: &str) -> bool {
        self[..].starts_with_ignore_ascii_case(other)
    }
}

pub static ALPHANUM_CHARS: LazyLock<Vec<char>> = LazyLock::new(|| {
    "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        .chars()
        .collect()
});

pub struct StringUtil;

impl StringUtil {
    pub fn rand(num: usize) -> String {
        let mut rng = rand::thread_rng();
        std::iter::repeat(())
            .map(|()| ALPHANUM_CHARS[rng.gen::<usize>() % ALPHANUM_CHARS.len()])
            .take(num)
            .collect()
    }

    pub fn rand_name(num: usize) -> String {
        if num == 0 {
            return "".to_string();
        }
        format!("_{}", Self::rand(num - 1))
    }
}

#[macro_export]
macro_rules! ssformat {
    ($len:expr, $($arg:tt)*) => {
        {
            use std::fmt::Write;
            let mut buf = smallstr::SmallString::<[u8; $len]>::new();
            buf.write_fmt(::core::format_args!($($arg)*)).unwrap();
            buf
        }
    };
}
