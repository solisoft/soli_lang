pub mod helpers;

use crate::interpreter::environment::Environment;

pub fn register_datetime_builtins(_env: &mut Environment) {
    // All datetime functionality is now provided by DateTime and Duration classes
    // These classes are registered via register_datetime_and_duration_classes()
}
