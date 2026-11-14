//! `with_role` and `current_role`: whether reads may go to a read replica
//! (see [`crate::db::roles`]).
//!
//! ```soli
//! with_role("reading") do
//!   Report.where({ month: month }).count   # a replica, even after a write
//! end
//! with_role("writing") do
//!   Post.find(id)                         # the primary, even on a GET
//! end
//! current_role()                           # "reading" or "writing"
//! ```
//!
//! `with_role` runs a block, so it is intercepted by both engines (it needs
//! the engine to call the block); the native here only reports misuse.

use crate::db::roles::{self, Role};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

const USAGE: &str =
    "with_role(role) expects \"reading\" or \"writing\" and a block: with_role(\"reading\") do … end";

/// The role `args` name.
pub fn role_from_args(args: &[Value]) -> Result<Role, String> {
    match args {
        [Value::String(name)] => Role::parse(name),
        _ => Err(USAGE.to_string()),
    }
}

/// Run `block` under `role`, then go back to the role before. The reads a
/// `grouped` block deferred run under the role they were made under.
pub fn with_role<T, E>(
    role: Role,
    block: impl FnOnce() -> Result<T, E>,
    error: impl Fn(String) -> E,
) -> Result<T, E> {
    super::model::batch::flush().map_err(&error)?;
    roles::scoped(role, || {
        let value = block()?;
        super::model::batch::flush().map_err(&error)?;
        Ok(value)
    })
}

pub fn register_db_role_builtins(env: &mut Environment) {
    env.define(
        "current_role".to_string(),
        Value::NativeFunction(NativeFunction::new("current_role", Some(0), |_| {
            Ok(Value::String(roles::current().as_str().into()))
        })),
    );
    // The engines run `with_role(role) do … end` themselves; reaching this
    // means there was no block.
    env.define(
        "with_role".to_string(),
        Value::NativeFunction(NativeFunction::new("with_role", None, |_| {
            Err(USAGE.to_string())
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_is_named_by_one_string() {
        assert_eq!(
            role_from_args(&[Value::String("reading".into())]).unwrap(),
            Role::Reading
        );
        assert_eq!(
            role_from_args(&[Value::String("writing".into())]).unwrap(),
            Role::Writing
        );
        assert!(role_from_args(&[Value::String("replica".into())])
            .unwrap_err()
            .contains("unknown database role"));
        assert!(role_from_args(&[]).is_err());
    }
}
