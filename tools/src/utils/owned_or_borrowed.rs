use std::ops::Deref;

/// This is similar to Cow, without the clone semantics
pub enum OwnedOrBorrowed<'a, T> {
    Owned(T),
    Borrowed(&'a T),
}

impl<'a, T> Deref for OwnedOrBorrowed<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        match &self {
            Self::Owned(t) => t,
            Self::Borrowed(t) => *t,
        }
    }
}
