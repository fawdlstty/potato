//! 平铺插入序头存储：请求/响应头表从 FxHashMap 迁移为线性扫描向量。
//!
//! 动机（2026-11-26 d1 热路径调查）：HTTP 头表典型 2~10 项，`FxHashMap`
//! 的每项哈希 + 探测 + 表清零成本远高于线性扫描，且响应构造/序列化两侧
//! 的反复查找放大了这一成本。平铺向量保留「insert 覆盖同名键、其余按
//! 插入序迭代」的原语义，序列化顺序从哈希随机序变为确定的插入序
//! （HTTP 头序本无语义约束，fuzz 断言两侧序列化一致已同步）。
//!
//! 本文件为 crate 内部 API：`HttpRequest::headers` / `HttpResponse::headers`
//! 公开字段类型随之变更（API break，2026-11-26 主人拍板追平 d1 性能）；
//! 公开方法面（get/add_header/iter 等）保持同名同签名语义。

use crate::utils::refstr::{HeaderItem, HeaderOrHipStr};
use hipstr::LocalHipStr;
use std::borrow::Cow;
use std::iter::FromIterator;

impl FromIterator<(Cow<'static, str>, Cow<'static, str>)> for ResponseHeaders {
    fn from_iter<T: IntoIterator<Item = (Cow<'static, str>, Cow<'static, str>)>>(iter: T) -> Self {
        let mut ret = Self::default();
        for (k, v) in iter {
            ret.insert(k, v);
        }
        ret
    }
}

impl FromIterator<(HeaderOrHipStr, LocalHipStr<'static>)> for RequestHeaders {
    fn from_iter<T: IntoIterator<Item = (HeaderOrHipStr, LocalHipStr<'static>)>>(iter: T) -> Self {
        let mut ret = Self::default();
        for (k, v) in iter {
            ret.insert(k, v);
        }
        ret
    }
}

/// 标准头槽位数：Date/Server/Connection/Content-Type/Pragma/Cache-Control/
/// Content-Encoding/Trailer/Content-Length/Transfer-Encoding。这些头出现在
/// 响应构造（default_headers）与序列化（write_to_stream/as_bytes 批路径）
/// 的每请求热路径上，掩码命中时 get/insert 免线性扫描直达 entries 下标。
const STD_SLOTS: usize = 10;

/// 标准头键识别：(长度, 小写首字节) 预筛——10 个标准头在此预筛下两两互异，
/// 命中后再做全名大小写不敏感校验兜底，非标准键至多 2 次比较即失败。
fn std_slot_of(key: &str) -> Option<usize> {
    let b = key.as_bytes();
    let first = b.first().map_or(0, |c| c.to_ascii_lowercase());
    let slot = match (b.len(), first) {
        (4, b'd') => 0,  // Date
        (6, b's') => 1,  // Server
        (10, b'c') => 2, // Connection
        (13, b'c')
            if b.len() > 1 && b[1].eq_ignore_ascii_case(&b'o') =>
        {
            3 // Content-Type
        }
        (13, b'c') => 5,     // Cache-Control
        (6, b'p') => 4,      // Pragma
        (16, b'c') => 6,     // Content-Encoding
        (7, b't') => 7,      // Trailer
        (14, b'c') => 8,     // Content-Length
        (18, b't') => 9,     // Transfer-Encoding
        _ => return None,
    };
    const NAMES: [&str; STD_SLOTS] = [
        "Date",
        "Server",
        "Connection",
        "Content-Type",
        "Pragma",
        "Cache-Control",
        "Content-Encoding",
        "Trailer",
        "Content-Length",
        "Transfer-Encoding",
    ];
    if NAMES[slot].eq_ignore_ascii_case(key) {
        Some(slot)
    } else {
        None
    }
}

/// 请求/响应通用平铺头表（响应侧键值对为 `Cow<'static, str>`）
#[derive(Clone, Debug, Default)]
pub struct ResponseHeaders {
    entries: Vec<(Cow<'static, str>, Cow<'static, str>)>,
    /// 标准头槽位直接索引（仅 std_mask 对应位有效时可信）
    std_idx: [u16; STD_SLOTS],
    std_mask: u16,
}

impl ResponseHeaders {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: Vec::with_capacity(cap),
            std_idx: [0; STD_SLOTS],
            std_mask: 0,
        }
    }

    /// 插入或覆盖：同名键（大小写不敏感）原地替换保持原位置，否则追加。
    /// 标准头走掩码直达，非标准键退回线性扫描。
    pub fn insert(
        &mut self,
        key: Cow<'static, str>,
        value: Cow<'static, str>,
    ) -> Option<Cow<'static, str>> {
        if let Some(slot) = std_slot_of(&key) {
            let bit = 1u16 << slot;
            if self.std_mask & bit != 0 {
                let i = self.std_idx[slot] as usize;
                return Some(std::mem::replace(&mut self.entries[i].1, value));
            }
            self.std_idx[slot] = self.entries.len() as u16;
            self.std_mask |= bit;
            self.entries.push((key, value));
            return None;
        }
        match self.find_pos(&key) {
            Some(i) => Some(std::mem::replace(&mut self.entries[i].1, value)),
            None => {
                self.entries.push((key, value));
                None
            }
        }
    }

    /// 大小写不敏感查找。标准键掩码直达，非标准键线性扫描。
    pub fn get(&self, key: &str) -> Option<&Cow<'static, str>> {
        if let Some(slot) = std_slot_of(key) {
            if self.std_mask & (1u16 << slot) != 0 {
                return Some(&self.entries[self.std_idx[slot] as usize].1);
            }
            return None;
        }
        self.find_pos(key).map(|i| &self.entries[i].1)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.find_pos(key).is_some()
    }

    pub fn remove(&mut self, key: &str) -> Option<Cow<'static, str>> {
        let i = self.find_pos(key)?;
        let (_, value) = self.entries.remove(i);
        // 被删位置之后的标准头下标整体左移一位，指向被删位的槽位失效
        for slot in 0..STD_SLOTS {
            let bit = 1u16 << slot;
            if self.std_mask & bit != 0 {
                let idx = self.std_idx[slot] as usize;
                if idx == i {
                    self.std_mask &= !bit;
                } else if idx > i {
                    self.std_idx[slot] = (idx - 1) as u16;
                }
            }
        }
        Some(value)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Cow<'static, str>, &Cow<'static, str>)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn retain<F: FnMut(&Cow<'static, str>, &mut Cow<'static, str>) -> bool>(&mut self, mut f: F) {
        self.entries.retain_mut(|(k, v)| f(k, v));
        // 元素下标整体变动，标准头槽位索引失效，按现存表重建（低频路径）
        self.std_mask = 0;
        for (i, (k, _)) in self.entries.iter().enumerate() {
            if let Some(slot) = std_slot_of(k) {
                let bit = 1u16 << slot;
                if self.std_mask & bit == 0 {
                    self.std_idx[slot] = i as u16;
                    self.std_mask |= bit;
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.std_mask = 0;
    }

    fn find_pos(&self, key: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|(k, _)| k.eq_ignore_ascii_case(key))
    }
}

/// 请求头平铺存储（键为 `HeaderOrHipStr`，值为 `LocalHipStr<'static>`）
#[derive(Clone, Debug, Default)]
pub struct RequestHeaders {
    entries: Vec<(HeaderOrHipStr, LocalHipStr<'static>)>,
}

impl RequestHeaders {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: Vec::with_capacity(cap),
        }
    }

    /// 插入或覆盖：同名键原地替换（保持 HashMap 语义：重复头后值生效），
    /// 否则追加。key 取具体类型（与原 FxHashMap::insert 同形状，
    /// 调用侧 `.into()` 目标唯一，无推断歧义）
    pub fn insert(
        &mut self,
        key: HeaderOrHipStr,
        value: LocalHipStr<'static>,
    ) -> Option<LocalHipStr<'static>> {
        let pos = self.entries.iter().position(|(k, _)| k == &key);
        match pos {
            Some(i) => Some(std::mem::replace(&mut self.entries[i].1, value)),
            None => {
                self.entries.push((key, value));
                None
            }
        }
    }

    /// 精确键查找（HeaderItem 判别值比较 / HipStr 内容比较）
    pub fn get(&self, key: &HeaderOrHipStr) -> Option<&LocalHipStr<'static>> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// 已知标准头快速查找：单次判别值扫描
    pub fn get_header_item(&self, key: HeaderItem) -> Option<&LocalHipStr<'static>> {
        self.entries
            .iter()
            .find(|(k, _)| matches!(k, HeaderOrHipStr::HeaderItem(i) if *i == key))
            .map(|(_, v)| v)
    }

    /// 大小写不敏感字符串查找：已知头先算判别值（单次），再单遍扫描
    pub fn get_by_str(&self, key: &str) -> Option<&LocalHipStr<'static>> {
        let known = HeaderItem::try_from_str(key);
        self.entries
            .iter()
            .find(|(k, _)| match k {
                HeaderOrHipStr::HeaderItem(item) => known.as_ref() == Some(item),
                HeaderOrHipStr::HipStr(s) => s.as_str().eq_ignore_ascii_case(key),
            })
            .map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &HeaderOrHipStr) -> bool {
        self.get(key).is_some()
    }

    pub fn remove(&mut self, key: &HeaderOrHipStr) -> Option<LocalHipStr<'static>> {
        let pos = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(pos).1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&HeaderOrHipStr, &LocalHipStr<'static>)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn retain<F: FnMut(&HeaderOrHipStr, &mut LocalHipStr<'static>) -> bool>(&mut self, mut f: F) {
        self.entries.retain_mut(|(k, v)| f(k, v));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn reserve(&mut self, additional: usize) {
        self.entries.reserve(additional);
    }
}
