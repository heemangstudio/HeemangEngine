//! A list kept by several layouts at once.

use std::sync::Arc;

/// A vector shared by the copies of the layout holding it: cloning shares
/// it, and changing one copy's list copies it first (through
/// [`std::ops::DerefMut`] too). Reads go through [`Vec`]'s. An empty list
/// takes no allocation.
pub struct Shared<T>(Option<Arc<Vec<T>>>);

impl<T> Shared<T> {
    /// The list to change, copied first when another layout shares it.
    pub fn make_mut(&mut self) -> &mut Vec<T>
    where
        T: Clone,
    {
        Arc::make_mut(self.0.get_or_insert_with(|| Arc::new(Vec::new())))
    }

    /// Drops the room a list not shared yet kept from growing, and the
    /// allocation of an empty one.
    pub fn shrink_to_fit(&mut self) {
        match &mut self.0 {
            Some(v) if v.is_empty() => self.0 = None,
            Some(v) => {
                if let Some(v) = Arc::get_mut(v) {
                    v.shrink_to_fit();
                }
            }
            None => {}
        }
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Shared(self.0.clone())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Eq> Eq for Shared<T> {}

impl<T> Default for Shared<T> {
    fn default() -> Self {
        Shared(None)
    }
}

impl<T> std::ops::Deref for Shared<T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Vec<T> {
        match &self.0 {
            Some(v) => v,
            None => const { &Vec::new() },
        }
    }
}

impl<T: Clone> std::ops::DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        self.make_mut()
    }
}

/// Kept without the room left over from growing.
impl<T> From<Vec<T>> for Shared<T> {
    fn from(mut v: Vec<T>) -> Self {
        if v.is_empty() {
            return Shared(None);
        }
        v.shrink_to_fit();
        Shared(Some(Arc::new(v)))
    }
}

impl<T> FromIterator<T> for Shared<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Vec::from_iter(iter).into()
    }
}

impl<'a, T> IntoIterator for &'a Shared<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T: Clone> IntoIterator for &'a mut Shared<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.make_mut().iter_mut()
    }
}
