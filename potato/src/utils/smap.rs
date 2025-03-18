use smallvec::SmallVec;
use std::collections::HashMap;
use std::hash::Hash;

#[derive(Clone)]
pub struct SMap<K, V>
where
    K: Eq + Hash + Ord,
{
    pub data: SmallVec<[(K, V); 64]>,
    pub ext_data: Option<HashMap<K, V>>,
}

impl<K, V> SMap<K, V>
where
    K: Eq + Hash + Ord,
{
    pub fn new() -> Self {
        Self {
            data: SmallVec::new(),
            ext_data: None,
        }
    }

    pub fn insert(&mut self, key: K, mut value: V) -> Option<V> {
        if let Some(ext_data) = &mut self.ext_data {
            ext_data.insert(key, value)
        } else if self.data.len() == 64 {
            let mut ext_data = HashMap::with_capacity(128);
            for (k, v) in self.data.drain(..) {
                ext_data.insert(k, v);
            }
            ext_data.insert(key, value)
        } else {
            match self.data.binary_search_by(|probe| probe.0.cmp(&key)) {
                Ok(index) => {
                    std::mem::swap(&mut value, &mut self.data[index].1);
                    Some(value)
                }
                Err(index) => {
                    self.data.insert(index, (key, value));
                    None
                }
            }
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        if let Some(ext_data) = &self.ext_data {
            ext_data.get(key)
        } else {
            match self.data.binary_search_by(|probe| probe.0.cmp(&key)) {
                Ok(index) => Some(&self.data[index].1),
                Err(_) => None,
            }
        }
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(ext_data) = &mut self.ext_data {
            ext_data.remove(key)
        } else {
            match self.data.binary_search_by(|probe| probe.0.cmp(&key)) {
                Ok(index) => Some(self.data.remove(index).1),
                Err(_) => None,
            }
        }
    }

    pub fn len(&self) -> usize {
        self.data.len() + self.ext_data.as_ref().map_or(0, HashMap::len)
    }

    pub fn iter(&self) -> SMapIter<'_, K, V> {
        let state = match &self.ext_data {
            Some(ext) => IterState::HashMap(ext.iter()),
            None => IterState::SmallVec(self.data.iter()),
        };
        SMapIter { inner: state }
    }

    pub fn keys(&self) -> SMapKeys<'_, K, V> {
        let state = match &self.ext_data {
            Some(ext) => IterState::HashMap(ext.iter()),
            None => IterState::SmallVec(self.data.iter()),
        };
        SMapKeys { inner: state }
    }

    pub fn values(&self) -> SMapValues<'_, K, V> {
        let state = match &self.ext_data {
            Some(ext) => IterState::HashMap(ext.iter()),
            None => IterState::SmallVec(self.data.iter()),
        };
        SMapValues { inner: state }
    }
}

enum IterState<'a, K, V> {
    SmallVec(std::slice::Iter<'a, (K, V)>),
    HashMap(std::collections::hash_map::Iter<'a, K, V>),
}

pub struct SMapIter<'a, K, V> {
    inner: IterState<'a, K, V>,
}

impl<'a, K, V> Iterator for SMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            IterState::SmallVec(iter) => {
                let item = iter.next()?;
                Some((&item.0, &item.1))
            }
            IterState::HashMap(iter) => iter.next(),
        }
    }
}

pub struct SMapKeys<'a, K, V> {
    inner: IterState<'a, K, V>,
}

impl<'a, K, V> Iterator for SMapKeys<'a, K, V> {
    type Item = &'a K;
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            IterState::SmallVec(iter) => Some(&iter.next()?.0),
            IterState::HashMap(iter) => iter.next().map(|(k, _)| k),
        }
    }
}

pub struct SMapValues<'a, K, V> {
    inner: IterState<'a, K, V>,
}

impl<'a, K, V> Iterator for SMapValues<'a, K, V> {
    type Item = &'a V;
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            IterState::SmallVec(iter) => Some(&iter.next()?.1),
            IterState::HashMap(iter) => iter.next().map(|(_, v)| v),
        }
    }
}
