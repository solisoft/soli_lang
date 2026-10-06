# Declarative, enum-backed state machines (`state_machine :field do … end`):
# events, transitions (single and array `from`), guards, before/after hooks,
# the generated `pay` / `pay!` / `can_pay?` / `<state>?` methods, reflection,
# and the checks run when the model loads. Only `pay!` touches the database.

enum OrderState
  Pending,
  Paid,
  Shipped,
  Delivered,
  Cancelled
end

enum OtherState
  Elsewhere
end

class SmOrder < Model
  enum_field(:status, OrderState)
  state_machine(:status) do
    initial(OrderState.Pending)

    event(:pay) do
      transition(from: OrderState.Pending, to: OrderState.Paid)
      guard(fn() { this.total > 0 })
    end

    event(:ship) do
      transition(from: OrderState.Paid, to: OrderState.Shipped)
    end

    event(:deliver) do
      transition(from: OrderState.Shipped, to: OrderState.Delivered)
    end

    event(:cancel) do
      transition(from: [OrderState.Pending, OrderState.Paid], to: OrderState.Cancelled)
    end

    # A `false` return from a before hook vetoes the transition.
    before_transition(to: OrderState.Shipped) do
      !this.block_ship
    end
    after_transition(to: OrderState.Paid) do
      this.receipt_sent = true
    end
  end
end

def new_order(state, total)
  order = SmOrder.new()
  order.status = state
  order.total = total
  order
end

# Misconfigured machines, declared on demand: each raises as its class loads.
def define_machine_without_initial
  class SmNoInitial < Model
    enum_field(:status, OrderState)
    state_machine(:status) do
      event(:pay) do
        transition(from: OrderState.Pending, to: OrderState.Paid)
      end
    end
  end
end

def define_machine_without_enum_field
  class SmNoEnumField < Model
    state_machine(:status) do
      initial(OrderState.Pending)
    end
  end
end

def define_machine_with_foreign_state
  class SmForeignState < Model
    enum_field(:status, OrderState)
    state_machine(:status) do
      initial(OtherState.Elsewhere)
    end
  end
end

describe("state machine") do
  describe("predicates") do
    test("the current state's predicate is true, the others false") do
      order = new_order(OrderState.Pending, 10)

      assert_eq(order.pending?, true)
      assert_eq(order.paid?, false)
      assert_eq(order.shipped?, false)
      assert_eq(order.cancelled?, false)
    end

    test("a predicate tracks the current state after a transition") do
      order = new_order(OrderState.Pending, 10)
      order.pay

      assert_eq(order.pending?, false)
      assert_eq(order.paid?, true)
    end
  end

  describe("initial state") do
    test("a freshly built record is in the initial state") do
      fresh = SmOrder.new()

      assert_eq(fresh.pending?, true)
      fresh.total = 3
      assert_eq(fresh.can_pay?, true)
    end

    test("a freshly built record has the initial state in its field") do
      pending("bug: `initial` is not written to the field: SmOrder.new().status is nil")

      assert_eq(SmOrder.new().status, OrderState.Pending)
    end
  end

  describe("can_X? queries") do
    test("true when the transition is legal and the guard passes") do
      assert_eq(new_order(OrderState.Pending, 10).can_pay?, true)
    end

    test("false when the guard fails") do
      assert_eq(new_order(OrderState.Pending, 0).can_pay?, false)
    end

    test("false when the transition is illegal from the current state") do
      assert_eq(new_order(OrderState.Shipped, 10).can_pay?, false)
    end

    test("false from a terminal state for every event") do
      order = new_order(OrderState.Delivered, 10)

      assert_eq([order.can_pay?, order.can_ship?, order.can_deliver?, order.can_cancel?],
        [false, false, false, false])
    end
  end

  describe("transitions") do
    test("a legal event mutates the state field and returns true") do
      order = new_order(OrderState.Pending, 10)

      assert_eq(order.pay, true)
      assert(order.status == OrderState.Paid)
      assert_eq(order.status.variant, "Paid")
    end

    test("assert_eq agrees with == on the state a transition wrote") do
      pending("bug: after order.pay, status == OrderState.Paid but assert_eq(status, OrderState.Paid) fails")
      order = new_order(OrderState.Pending, 10)
      order.pay

      assert_eq(order.status, OrderState.Paid)
    end

    test("array `from` allows the event from several states") do
      from_pending = new_order(OrderState.Pending, 10)
      from_pending.cancel
      from_paid = new_order(OrderState.Paid, 10)
      from_paid.cancel

      assert_eq(from_pending.cancelled?, true)
      assert_eq(from_paid.cancelled?, true)
    end

    test("events chain through the whole lifecycle") do
      order = new_order(OrderState.Pending, 10)
      order.pay
      order.ship
      order.deliver

      assert_eq(order.delivered?, true)
    end

    test("an illegal transition raises and leaves the state alone") do
      order = new_order(OrderState.Shipped, 10)

      assert_raises("SmOrder: cannot 'pay' from state 'Shipped'") do
        order.pay
      end
      assert_eq(order.shipped?, true)
    end

    test("a failed guard raises on the event") do
      order = new_order(OrderState.Pending, 0)

      assert_raises("SmOrder: guard for 'pay' failed") do
        order.pay
      end
      assert_eq(order.pending?, true)
    end
  end

  describe("hooks") do
    test("after_transition runs on entering the target state") do
      order = new_order(OrderState.Pending, 10)
      order.pay

      assert_eq(order.receipt_sent, true)
    end

    test("after_transition does not run for another target state") do
      order = new_order(OrderState.Pending, 10)
      order.cancel

      assert_null(order.receipt_sent)
    end

    test("a before_transition returning false vetoes the transition") do
      order = new_order(OrderState.Paid, 10)
      order.block_ship = true

      assert_raises("SmOrder: before_transition to 'Shipped' vetoed 'ship'") do
        order.ship
      end
      assert_eq(order.paid?, true)
    end

    test("a before_transition returning true allows the transition") do
      order = new_order(OrderState.Paid, 10)
      order.block_ship = false
      order.ship

      assert_eq(order.shipped?, true)
    end
  end

  describe("reflection") do
    test("Model.events() lists the declared events in order") do
      assert_eq(SmOrder.events(), ["pay", "ship", "deliver", "cancel"])
    end

    test("Model.states() lists the enum's states") do
      assert_eq(SmOrder.states(), ["Pending", "Paid", "Shipped", "Delivered", "Cancelled"])
    end
  end

  describe("checks when the model loads") do
    test("a machine without `initial` raises") do
      assert_raises("state machine for SmNoInitial has no `initial` state") do
        define_machine_without_initial()
      end
    end

    test("a machine without a matching enum_field raises") do
      assert_raises("state_machine(:status) requires `enum_field :status, <Enum>` declared first") do
        define_machine_without_enum_field()
      end
    end

    test("a state from another enum raises") do
      assert_raises("'Elsewhere' is not a variant of enum OrderState") do
        define_machine_with_foreign_state()
      end
    end
  end

  describe("persistence") do
    before_each() do
      requires_solidb()
    end

    after_each() do
      SmOrder.delete_all()
    end

    test("pay! persists the new state") do
      order = SmOrder.create({"status": OrderState.Pending, "total": 5})

      assert_eq(order.pay!, true)
      assert_eq(SmOrder.find(order._key).paid?, true)
    end

    test("pay without the bang changes nothing in the database") do
      order = SmOrder.create({"status": OrderState.Pending, "total": 5})
      order.pay

      assert_eq(order.paid?, true)
      assert_eq(SmOrder.find(order._key).pending?, true)
    end

    test("an illegal pay! raises and persists nothing") do
      order = SmOrder.create({"status": OrderState.Shipped, "total": 5})

      assert_raises("cannot 'pay' from state 'Shipped'") do
        order.pay!
      end
      assert_eq(SmOrder.find(order._key).shipped?, true)
    end
  end
end
