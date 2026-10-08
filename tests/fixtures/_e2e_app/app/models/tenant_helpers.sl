# Application functions a tenant scope calls (see `GadgetScopes`). Here, not in
# app/helpers: those are view helpers, which models never see.
const TENANT_PREFIX = "acme"

def current_tenant()
  TENANT_PREFIX + "-eu"
end

def normalize_tag(tag)
  tag.downcase.trim
end
