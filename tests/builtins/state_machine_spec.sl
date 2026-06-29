# ============================================================================
# Declarative, enum-backed state machines (`state_machine :field do … end`).
#
# Covers the full DSL surface: events, transitions (single + array `from`),
# guards, before/after hooks, generated event methods (`pay`, `pay!`),
# `can_X?` queries, `<state>?` predicates, illegal-transition raising, and
# reflection. The persistence path (`pay!`) is gated on DB availability.
# ============================================================================

enum OrderState
  Pending,
  Paid,
  Shipped,
  Delivered,
  Cancelled
end

class SmOrder < Model
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

    event :deliver do
      transition from: OrderState.Shipped, to: OrderState.Delivered
    end

    event :cancel do
      transition from: [OrderState.Pending, OrderState.Paid], to: OrderState.Cancelled
    end

    # A `false` return from a before hook vetoes the transition.
    before_transition to: OrderState.Shipped do !this.block_ship end
    after_transition  to: OrderState.Paid do this.receipt_sent = true end
  end
end

# Local helper — there is no global assert_throws builtin.
fn assert_throws(body) {
  let threw = false
  try {
    body()
  } catch e {
    threw = true
  }
  assert(threw)
}

fn new_order(state, total) {
  let order = SmOrder.new()
  order.status = state
  order.total = total
  return order
}

describe("state machine — predicates", fn() {
  test("initial-state predicate is true, others false", fn() {
    let order = new_order(OrderState.Pending, 10)
    assert_eq(order.pending?, true)
    assert_eq(order.paid?, false)
    assert_eq(order.shipped?, false)
  })

  test("predicate tracks the current state after a transition", fn() {
    let order = new_order(OrderState.Pending, 10)
    order.pay
    assert_eq(order.pending?, false)
    assert_eq(order.paid?, true)
  })
})

describe("state machine — can_X? queries", fn() {
  test("true when the transition is legal and the guard passes", fn() {
    let order = new_order(OrderState.Pending, 10)
    assert_eq(order.can_pay?, true)
  })

  test("false when the guard fails", fn() {
    let order = new_order(OrderState.Pending, 0)
    assert_eq(order.can_pay?, false)
  })

  test("false when the transition is illegal from the current state", fn() {
    let order = new_order(OrderState.Shipped, 10)
    assert_eq(order.can_pay?, false)
  })
})

describe("state machine — transitions", fn() {
  test("a legal event mutates the state field", fn() {
    let order = new_order(OrderState.Pending, 10)
    order.pay
    assert_eq(order.status.variant(), "Paid")
  })

  test("array `from` allows the event from several states", fn() {
    let from_pending = new_order(OrderState.Pending, 10)
    from_pending.cancel
    assert_eq(from_pending.cancelled?, true)

    let from_paid = new_order(OrderState.Paid, 10)
    from_paid.cancel
    assert_eq(from_paid.cancelled?, true)
  })

  test("an illegal transition raises", fn() {
    let order = new_order(OrderState.Shipped, 10)
    assert_throws(fn() { order.pay })
  })

  test("a failed guard raises on the event", fn() {
    let order = new_order(OrderState.Pending, 0)
    assert_throws(fn() { order.pay })
  })
})

describe("state machine — hooks", fn() {
  test("after_transition runs on entering the target state", fn() {
    let order = new_order(OrderState.Pending, 10)
    order.pay
    assert_eq(order.receipt_sent, true)
  })

  test("a before_transition returning false vetoes the transition", fn() {
    let order = new_order(OrderState.Paid, 10)
    order.block_ship = true
    assert_throws(fn() { order.ship })
    assert_eq(order.shipped?, false)
  })

  test("a before_transition returning true allows the transition", fn() {
    let order = new_order(OrderState.Paid, 10)
    order.block_ship = false
    order.ship
    assert_eq(order.shipped?, true)
  })
})

describe("state machine — reflection", fn() {
  test("Model.events lists declared events", fn() {
    let events = SmOrder.events()
    assert(events.includes?("pay"))
    assert(events.includes?("cancel"))
  })

  test("Model.states lists referenced states", fn() {
    let states = SmOrder.states()
    assert(states.includes?("Pending"))
    assert(states.includes?("Delivered"))
  })
})

# --- Persistence path (requires a DB) -------------------------------------
let __db_available = false
try
  let __probe = SmOrder.create({ "status": OrderState.Pending, "total": 1 })
  if !__probe.nil? and !__probe._errors
    __db_available = true
    __probe.delete()
  end
catch e
end

if __db_available
describe("state machine — persistence", fn() {
  test("pay! persists the new state to the database", fn() {
    let order = SmOrder.create({ "status": OrderState.Pending, "total": 5 })
    order.pay!
    let reloaded = SmOrder.find(order._key)
    assert_eq(reloaded.paid?, true)
    order.delete()
  })
})
end
