use std::{collections::HashMap, hash::Hash};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BoundedRing<T, const N: usize> {
    items: Vec<T>,
}

struct ExpectedAtMost<const N: usize>;

impl<const N: usize> serde::de::Expected for ExpectedAtMost<N> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "expected at most {N}")
    }
}

impl<T: Clone + Eq + Hash, const N: usize> BoundedRing<T, N> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, item: T) {
        if N == 0 {
            return;
        }
        if self.items.len() == N {
            self.items.remove(0);
        }
        self.items.push(item);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    pub fn most_common_count_in(&self, window: usize) -> usize {
        if window == 0 || self.items.is_empty() {
            return 0;
        }
        let window = window.min(self.items.len());
        let mut counts: HashMap<&T, usize> = HashMap::new();
        for item in self.items[self.items.len() - window..].iter() {
            *counts.entry(item).or_insert(0) += 1;
        }
        counts.values().copied().max().unwrap_or(0)
    }

    pub fn same_run_length(&self) -> usize {
        let Some(last) = self.items.last() else {
            return 0;
        };
        self.items
            .iter()
            .rev()
            .take_while(|item| *item == last)
            .count()
    }
}

impl<T, const N: usize> Default for BoundedRing<T, N> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<'de, T: serde::Deserialize<'de>, const N: usize> serde::Deserialize<'de>
    for BoundedRing<T, N>
{
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(bound(deserialize = "T: serde::Deserialize<'de>"))]
        struct Inner<T> {
            items: Vec<T>,
        }

        let raw: Inner<T> = Inner::deserialize(deserializer)?;
        if raw.items.len() > N {
            return Err(serde::de::Error::invalid_length(
                raw.items.len(),
                &ExpectedAtMost::<N>,
            ));
        }
        Ok(Self { items: raw.items })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_rejects_items_longer_than_capacity() {
        let result = serde_json::from_str::<BoundedRing<u32, 2>>(r#"{"items":[1,2,3]}"#);

        assert!(result.is_err());
    }

    #[test]
    fn deserialize_accepts_items_at_capacity() {
        let ring = serde_json::from_str::<BoundedRing<u32, 2>>(r#"{"items":[1,2]}"#).unwrap();

        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn deserialize_accepts_items_below_capacity() {
        let ring = serde_json::from_str::<BoundedRing<u32, 2>>(r#"{"items":[1]}"#).unwrap();

        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), vec![1]);
    }
}
