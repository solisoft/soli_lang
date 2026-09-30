# Policies

Authorization policies. A policy is a class `<Model>Policy < ApplicationPolicy`
with one predicate per action: `index?`, `show?`, `create?`, `new?`,
`update?`, `edit?`, `destroy?`.

Files here are auto-loaded into the global scope at boot (like models), so
controllers can call `authorize(...)` and `const_get("<Model>Policy")` resolves
the class. Restart the server after adding a new policy.

## Using a policy in a controller

```soli
def update
  post = Post.find(params["id"])
  authorize(post)            # 403 unless PostPolicy#update? is true
  post.update(this._permit_params(params))
  return redirect(post_path(post))
end
```

`authorize(record)` infers the action from the current request. Pass an
explicit one with `authorize(record, "show")`.

## Writing a policy

```soli
class PostPolicy < ApplicationPolicy
  def show?
    true                                  # anyone may read
  end

  def update?
    return false unless signed_in?()      # `this.user` is the current user

    return this.user["_key"] == this.record["author_id"]
  end
end
```

Policies default to **deny** — override only what you want to allow. A record
class with no matching policy is denied (403), so a forgotten policy fails
closed. See `application_policy.sl` for the base class and `user_policy.sl` for
a worked example.
