//! `Policy`: the deny-by-default authorization base.
//!
//! `soli generate auth` used to copy every predicate into `ApplicationPolicy`.
//! The predicates now live here, so a new app's `ApplicationPolicy < Policy`
//! only keeps the constructor. An app that already copied the old class is
//! unchanged: nothing it defines is named `Policy`.

use crate::interpreter::Interpreter;

/// Loaded before application code, the way `Mailer` is.
pub const POLICY_PRELUDE: &str = r#"
class Policy
  user: Any

  def index?
    false
  end

  def show?
    false
  end

  def create?
    false
  end

  def new?
    @create?
  end

  def update?
    false
  end

  def edit?
    @update?
  end

  def destroy?
    false
  end

  def signed_in?
    !@user.nil?
  end
end
"#;

/// Define `Policy` when this interpreter does not have it yet.
pub fn ensure_prelude(interpreter: &mut Interpreter) {
    if interpreter.global_env().borrow().get("Policy").is_some() {
        return;
    }
    let tokens = match crate::lexer::Scanner::new(POLICY_PRELUDE).scan_tokens() {
        Ok(tokens) => tokens,
        Err(error) => {
            eprintln!("policy prelude lex error: {error}");
            return;
        }
    };
    let program = match crate::parser::Parser::new(tokens).parse() {
        Ok(program) => program,
        Err(error) => {
            eprintln!("policy prelude parse error: {error}");
            return;
        }
    };
    if let Err(error) = interpreter.interpret(&program) {
        eprintln!("policy prelude execute error: {error}");
    }
}
