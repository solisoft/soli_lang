# State Machines

Drive an `enum_field` column through a declarative, enum-backed state machine declared right inside your model class.

## Overview

A `state_machine` block models a workflow as a set of discrete states (the variants of an `enum`) and the legal `transition`s between them. You declare it in the model class body — alongside `validates`, `scope`, and the lifecycle callbacks — using the same class-body DSL style.

Soli generates the runtime surface for you: an event method per declared event (`pay`, `pay!`, `can_pay?`), a boolean predicate per state (`pending?`, `paid?`, …), and class-level reflection (`Order.events()`, `Order.states()`). Illegal moves and failed guards raise; the `can_X?` predicates never raise, so they're safe to drive UI and conditionals.

## A Complete Example

```soli
# app/models/order.sl
enum OrderState
  Pending,
  Paid,
  Shipped,
  Delivered,
  Cancelled
end

class Order < Model
  enum_field :status, OrderState

  state_machine :status do
    initial OrderState.Pending

    event :pay do
      transition from: OrderState.Pending, to: OrderState.Paid
      guard fn() { this.total > 0 }
    end

    event :ship do
      transition from: OrderState.Paid, to: OrderState.Shipped
    end

    event :cancel do
      transition from: [OrderState.Pending, OrderState.Paid], to: OrderState.Cancelled
    end

    before_transition to: OrderState.Shipped do this.warehouse_ready? end
    after_transition  to: OrderState.Paid do this.send_receipt() end
  end
end
```

## The DSL

### `state_machine :field do … end`

Declares a machine that drives an `enum_field` column. The `:field` symbol must match a field already declared with `enum_field :field, SomeEnum` **earlier in the class body** — the machine's states are exactly that enum's variants.

```soli
class Order < Model
  enum_field :status, OrderState   # declare the column first

  state_machine :status do
    # …
  end
end
```

### `initial EnumType.Variant`

The starting state of a freshly built record. **Required** — omitting it raises a clear error when the model loads.

```soli
initial OrderState.Pending
```

### `event :name do … end`

Declares an event. An event holds one or more `transition`s and an optional `guard`. The event name becomes the generated method (`:pay` → `pay` / `pay!` / `can_pay?`).

```soli
event :pay do
  transition from: OrderState.Pending, to: OrderState.Paid
  guard fn() { this.total > 0 }
end
```

### `transition from: X, to: Y`

A legal move. `from:` accepts a single state **or** an array of states; `to:` is always a single state. List multiple `transition`s inside one event when several source states converge on the same target.

```soli
# Single source
transition from: OrderState.Paid, to: OrderState.Shipped

# Multiple sources, one target
transition from: [OrderState.Pending, OrderState.Paid], to: OrderState.Cancelled
```

### `guard fn() { … }`

A predicate run with `this` bound to the record. The event fires (and `can_X?` returns `true`) only when the guard returns a truthy value. A guard is optional; an event without one is gated solely by the legality of its transitions.

```soli
event :pay do
  transition from: OrderState.Pending, to: OrderState.Paid
  guard fn() { this.total > 0 }
end
```

### `before_transition` / `after_transition`

Hooks keyed by the state being entered, both run with `this` bound to the record.

- `before_transition to: X do … end` runs **before** entering state `X`. **Returning `false` vetoes the transition** — the move is aborted and the event raises.
- `after_transition to: X do … end` runs **after** the record has entered state `X`.

```soli
before_transition to: OrderState.Shipped do this.warehouse_ready? end
after_transition  to: OrderState.Paid do this.send_receipt() end
```

## Generated Methods

For an event named `pay` and a state enum with variants `Pending`, `Paid`, … Soli generates:

| Method | Kind | Behavior |
|--------|------|----------|
| `order.pay` | event | Performs the transition in memory: checks legality, runs the guard and before/after hooks, sets the field. **Raises** on an illegal transition or a failed guard. |
| `order.pay!` | event (persisting) | Same as `order.pay`, then **persists** via the record's normal `save` path. |
| `order.can_pay?` | predicate | `true` only if the transition is legal from the current state **and** the guard (if any) passes. **Never raises.** |
| `order.pending?`, `order.paid?`, … | state predicate | One per enum variant, snake_cased (`InTransit` → `in_transit?`). Pure boolean check of the current state. |
| `Order.events()` | reflection | Array of the event-name strings. |
| `Order.states()` | reflection | Array of the state-tag strings. |

## Raise vs. `can_X?`

Event methods are strict: `pay` and `pay!` **raise** on an illegal transition or a failed guard. The matching `can_pay?` predicate **never raises** — it returns `false` instead. Use the predicate to branch, and reserve the event call for the path you've already confirmed is legal.

```soli
# Idiomatic: ask first, then act
if order.can_pay?
  order.pay!
else
  # legal-but-unmet guard, or wrong source state
  flash[:error] = "This order can't be paid yet."
end
```

Calling the event without checking is fine when an illegal move is genuinely exceptional — the raise surfaces it loudly:

```soli
order.ship   # raises if the order isn't Paid (or warehouse_ready? returns false)
```

## Persisting with `!`

The plain event method (`pay`) mutates the record **in memory only**. The bang form (`pay!`) performs the same transition and then saves through the record's normal `save` path — so model validations and lifecycle callbacks fire exactly as they would for any other write.

```soli
order.pay    # field becomes Paid in memory; nothing written yet
order.save   # persist later, by hand

order.pay!   # transition + persist in one step
```

## Reflection

Introspect the machine at the class level:

```soli
Order.events()   # ["pay", "ship", "cancel"]
Order.states()   # ["Pending", "Paid", "Shipped", "Delivered", "Cancelled"]
```

## Validation at Boot

The machine is validated when the model loads (at server boot, and under `soli test`). Each of these raises a clear error before any request runs, so misconfiguration never reaches production:

- Referencing a variant that isn't part of the field's enum.
- Omitting the required `initial` state.
- Declaring `state_machine :field` without a matching `enum_field :field, …` earlier in the class body.

## Production Note

Under the production VM, state machine event methods transparently fall back to the tree-walking interpreter — the same mechanism model lifecycle callbacks use. Behavior is identical in development and production; no user action is required.
