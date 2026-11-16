use std::rc::Rc;

use rapier2d::prelude::*;

use super::world::Shared;

/// A connection between two bodies, the equivalent of LÖVE's `Joint`.
///
/// Make one with the `new_..._joint` methods of
/// [`World`](super::World). It stays in place until destroyed, or until
/// one of its bodies is.
#[derive(Clone)]
pub struct Joint {
    world: Shared,
    handle: ImpulseJointHandle,
}

impl PartialEq for Joint {
    fn eq(&self, other: &Joint) -> bool {
        self.handle == other.handle && Rc::ptr_eq(&self.world, &other.world)
    }
}

impl Joint {
    pub(super) fn new(world: &Shared, handle: ImpulseJointHandle) -> Joint {
        Joint {
            world: world.clone(),
            handle,
        }
    }

    /// Whether the joint, or one of its bodies, has been destroyed.
    pub fn is_destroyed(&self) -> bool {
        self.world
            .borrow()
            .world
            .impulse_joints
            .get(self.handle)
            .is_none()
    }

    /// Removes the joint, setting its bodies free of each other. Does
    /// nothing if it was already destroyed.
    pub fn destroy(&self) {
        self.world
            .borrow_mut()
            .world
            .remove_impulse_joint(self.handle);
    }
}

impl std::fmt::Debug for Joint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Joint")
            .field("destroyed", &self.is_destroyed())
            .finish()
    }
}
