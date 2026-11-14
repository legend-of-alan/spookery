//! The stories' components, keyed by story id. The pack's stories all exist; the save holds
//! only what is on them.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::state::component::{self, Meter};
use crate::state::id::StoryId;
use crate::state::{entity, nested_map, undo};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Stories {
    /// The meters each story file declares.
    #[serde(with = "nested_map")]
    pub(in crate::state) meters: undo::Store<component::Map<StoryId, Meter>>,
    #[serde(with = "nested_map::lists")]
    pub(in crate::state) flags: undo::Store<component::Map<StoryId, component::Flag>>,
}

impl entity::Entities for Stories {
    type Id = StoryId;

    fn contains(&self, canon: &Canon, id: &StoryId) -> bool {
        canon.stories.contains_key(id)
    }

    fn ids<'a>(&'a self, canon: &'a Canon) -> entity::query::Ids<'a, Self> {
        Box::new(canon.stories.keys())
    }
}

entity::has!(Stories, Meter, meters);
entity::has!(Stories, component::Flag, flags);

impl Stories {
    /// Every store, as the log a rollback undoes and an accept clears.
    pub(in crate::state) fn logs_mut(&mut self) -> [&mut dyn undo::Log; 2] {
        // Every store, with no `..`: a store added later must be listed here to compile.
        let Stories { meters, flags } = self;
        [meters, flags]
    }
}
