//! The item counts actors and places hold.

use crate::model::tracked::{Edit, Store};

/// Every item count, on every actor and place, by the item's id. A holder with none of an item
/// has no count for it: putting the first one in adds the count, and taking the last one out
/// takes it off.
pub type Items = Store<u32, Shift>;

/// A count moves by this much, to a count above 0 the writer has checked. Its undo moves it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shift(pub(crate) i64);

impl Edit<u32> for Shift {
    fn apply(self, count: &mut u32) -> Option<Self> {
        if self.0 == 0 {
            return None;
        }
        *count = u32::try_from(i64::from(*count).checked_add(self.0)?).ok()?;
        Some(Shift(-self.0))
    }
}
