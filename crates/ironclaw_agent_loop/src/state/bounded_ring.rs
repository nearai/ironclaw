use std::{collections::HashMap, hash::Hash, marker::PhantomData};

use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};

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

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
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
        enum Field {
            Items,
            Ignored,
        }

        impl<'de> serde::Deserialize<'de> for Field {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct FieldVisitor;

                impl<'de> Visitor<'de> for FieldVisitor {
                    type Value = Field;

                    fn expecting(
                        &self,
                        formatter: &mut std::fmt::Formatter<'_>,
                    ) -> std::fmt::Result {
                        formatter.write_str("`items`")
                    }

                    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Field, E> {
                        Ok(match value {
                            "items" => Field::Items,
                            _ => Field::Ignored,
                        })
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct BoundedRingVisitor<T, const N: usize> {
            item: PhantomData<T>,
        }

        impl<'de, T: serde::Deserialize<'de>, const N: usize> Visitor<'de> for BoundedRingVisitor<T, N> {
            type Value = BoundedRing<T, N>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a bounded ring with an items array")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut items = None;
                while let Some(field) = map.next_key::<Field>()? {
                    match field {
                        Field::Items => {
                            if items.is_some() {
                                return Err(serde::de::Error::duplicate_field("items"));
                            }
                            items = Some(map.next_value_seed(BoundedItemsVisitor::<T, N> {
                                item: PhantomData,
                            })?);
                        }
                        Field::Ignored => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                let items = items.ok_or_else(|| serde::de::Error::missing_field("items"))?;
                Ok(BoundedRing { items })
            }
        }

        struct BoundedItemsVisitor<T, const N: usize> {
            item: PhantomData<T>,
        }

        impl<'de, T: serde::Deserialize<'de>, const N: usize> serde::de::DeserializeSeed<'de>
            for BoundedItemsVisitor<T, N>
        {
            type Value = Vec<T>;

            fn deserialize<D: serde::Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<Self::Value, D::Error> {
                deserializer.deserialize_seq(self)
            }
        }

        impl<'de, T: serde::Deserialize<'de>, const N: usize> Visitor<'de> for BoundedItemsVisitor<T, N> {
            type Value = Vec<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "an array with at most {N} items")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(N));
                while items.len() < N {
                    let Some(item) = seq.next_element::<T>()? else {
                        return Ok(items);
                    };
                    items.push(item);
                }
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(
                        N + 1,
                        &ExpectedAtMost::<N>,
                    ));
                }
                Ok(items)
            }
        }

        deserializer.deserialize_struct(
            "BoundedRing",
            &["items"],
            BoundedRingVisitor::<T, N> { item: PhantomData },
        )
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
    fn deserialize_rejects_capacity_before_deserializing_extra_typed_item() {
        let error = serde_json::from_str::<BoundedRing<u32, 2>>(r#"{"items":[1,2,"ignored"]}"#)
            .unwrap_err();

        assert!(error.to_string().contains("expected at most 2"));
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
