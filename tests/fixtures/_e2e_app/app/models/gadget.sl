# A model whose scopes come from a module's `included` block — the shape an
# app uses for a shared tenant scope. `ScopesController` reads them under
# `soli serve`, where actions run on the bytecode VM.
module GadgetScopes
  included do
    scope("owned", fn() { this.where({"owner": "o1"}) })
    scope("of_kind", fn(kind) { this.where({"kind": kind}) })
    # Scopes that call application helpers and read a constant, as a tenant
    # scope does: their bodies resolve those names in their own closure.
    scope("for_tenant", fn() { this.where({"tenant": current_tenant()}) })
    scope("tagged", fn(tag) { this.where({"tag": normalize_tag(tag)}) })
  end
end

class Gadget < Model
  include GadgetScopes

  static def label() -> String
    return "gadget"
  end
end
