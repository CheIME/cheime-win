//! TIP instance — the text input processor object.
//!
//! Created by `IClassFactory::CreateInstance`. Holds the I/O thread,
//! candidate window, and channel dispatch state.

use crate::exports::increment_object_count;
use std::sync::atomic::{AtomicU32, Ordering};

/// CheIME TIP — the object that TSF interacts with.
pub struct CheimeTip {
    pub ref_count: AtomicU32,
}

impl CheimeTip {
    pub fn new() -> Box<Self> {
        increment_object_count();
        Box::new(Self {
            ref_count: AtomicU32::new(1),
        })
    }

    pub fn add_ref(&self) -> u32 {
        self.ref_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn release(&self) -> u32 {
        let prev = self.ref_count.fetch_sub(1, Ordering::Relaxed);
        prev - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_new_has_ref_count_one() {
        let tip = CheimeTip::new();
        assert_eq!(tip.ref_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn add_ref_and_release() {
        let tip = CheimeTip::new();
        assert_eq!(tip.add_ref(), 2);
        assert_eq!(tip.release(), 1);
    }
}
