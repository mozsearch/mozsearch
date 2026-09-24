use std::iter::Flatten;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum OverflowingVecImpl<const MAX_SIZE: usize, T> {
    InBounds(Vec<T>),
    Overflow(usize),
}

/// Either a Vec of size <= MAX_SIZE, or just a size > MAX_SIZE.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OverflowingVec<const MAX_SIZE: usize, T>(OverflowingVecImpl<MAX_SIZE, T>);

impl<const MAX_SIZE: usize, T> From<Vec<T>> for OverflowingVec<MAX_SIZE, T> {
    fn from(vec: Vec<T>) -> Self {
        if vec.len() > MAX_SIZE {
            return Self(OverflowingVecImpl::Overflow(vec.len()));
        }

        Self(OverflowingVecImpl::InBounds(vec))
    }
}

impl<const MAX_SIZE: usize, T> TryFrom<OverflowingVec<MAX_SIZE, T>> for Vec<T> {
    type Error = usize;

    fn try_from(value: OverflowingVec<MAX_SIZE, T>) -> Result<Self, Self::Error> {
        match value.0 {
            OverflowingVecImpl::InBounds(vec) => {
                if vec.len() > MAX_SIZE {
                    // Can happen because from deserialization.
                    Err(vec.len())
                } else {
                    Ok(vec)
                }
            }
            OverflowingVecImpl::Overflow(size) => Err(size),
        }
    }
}

impl<const MAX_SIZE: usize, T> OverflowingVec<MAX_SIZE, T> {
    pub fn is_empty(&self) -> bool {
        match self.0 {
            OverflowingVecImpl::InBounds(ref items) => items.is_empty(),
            OverflowingVecImpl::Overflow(size) => {
                // Can happen because from deserialization.
                size == 0
            }
        }
    }
}

impl<const MAX_SIZE: usize, T> Default for OverflowingVec<MAX_SIZE, T> {
    fn default() -> Self {
        Self(OverflowingVecImpl::InBounds(vec![]))
    }
}

impl<'a, const MAX_SIZE: usize, T> IntoIterator for &'a OverflowingVec<MAX_SIZE, T> {
    type Item = &'a T;

    type IntoIter = Flatten<<Option<&'a Vec<T>> as IntoIterator>::IntoIter>;

    fn into_iter(self) -> Self::IntoIter {
        match &self.0 {
            OverflowingVecImpl::InBounds(items) => Some(items).into_iter().flatten(),
            OverflowingVecImpl::Overflow(_) => None.into_iter().flatten(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OverflowingVec;
    use serde_json;

    #[test]
    fn serialization() {
        let just_right = OverflowingVec::<3, u32>::from(vec![1, 2, 3]);
        assert_eq!(serde_json::to_string(&just_right).unwrap(), "[1,2,3]");

        let overflow = OverflowingVec::<3, u32>::from(vec![1, 2, 3, 4]);
        assert_eq!(serde_json::to_string(&overflow).unwrap(), "4");
    }

    #[test]
    fn deserialization() {
        let just_right: OverflowingVec<3, u32> = serde_json::from_str("[1, 2, 3]").unwrap();
        assert_eq!(just_right.try_into(), Ok(vec![1, 2, 3]));

        let too_big: OverflowingVec<3, u32> = serde_json::from_str("[1, 2, 3, 4]").unwrap();
        assert_eq!(too_big.try_into(), Err::<Vec<_>, usize>(4));

        let overflow: OverflowingVec<3, u32> = serde_json::from_str("5").unwrap();
        assert_eq!(overflow.try_into(), Err::<Vec<_>, usize>(5));

        let false_overflow: OverflowingVec<10, u32> = serde_json::from_str("6").unwrap();
        assert_eq!(false_overflow.try_into(), Err::<Vec<_>, usize>(6));
    }
}
