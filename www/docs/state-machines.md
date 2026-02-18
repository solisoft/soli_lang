# State Machines

Implement state machines for managing complex workflows and business logic in your application.

## Overview

State machines provide a structured way to model workflows with discrete states and transitions. They help prevent invalid state transitions and make complex business logic more maintainable.

## Creating a State Machine

Use the `create_state_machine()` function to create a new state machine instance:

```soli
let states = ["pending", "confirmed", "processing", "shipped", "delivered", "cancelled"];
let transitions = [
    {"event": "confirm", "from": "pending", "to": "confirmed"},
    {"event": "process", "from": "confirmed", "to": "processing"},
    {"event": "ship", "from": "processing", "to": "shipped"},
    {"event": "deliver", "from": "shipped", "to": "delivered"},
    {"event": "cancel", "from": "pending", "to": "cancelled"}
];

let order = create_state_machine("pending", states, transitions);
```

### Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `initial_state` | String | The starting state (must be in the states array) |
| `states` | Array | Array of valid state names |
| `transitions` | Array | Array of transition definitions |

### Transition Definition

Each transition can have these keys:

| Key | Description |
|-----|-------------|
| `event` | Name of the event/method to create |
| `from` | Source state(s) - can be a string or array of strings |
| `to` | Target state |
| `if` | Optional guard condition field name |
| `guard` | Optional guard condition field name (alias for `if`) |

## Guard Conditions

Guard conditions allow you to conditionally enable transitions based on context values:

```soli
let transitions = [
    {"event": "confirm", "from": "pending", "to": "confirmed"},
    {"event": "ship", "from": "processing", "to": "shipped", "if": "can_ship"},
    {"event": "deliver", "from": "shipped", "to": "delivered", "guard": "is_deliverable"},
    {"event": "cancel", "from": "pending", "to": "cancelled"}
];

let order = create_state_machine("pending", states, transitions);

// Set guard conditions
order.set("can_ship", true);
order.set("is_deliverable", true);

// Transitions will fail if guard is false
order.set("can_ship", false);
order.ship();  // Error: Guard condition 'can_ship' is false
```

The `if` and `guard` keys specify a context field that must be `true` (or undefined) for the transition to proceed.

## Advanced Methods

### last_transition()

Returns information about the last transition:

```soli
order.confirm();
let last = order.last_transition();
// {from => pending, to => confirmed, event => confirm}
```

### can(event)

Checks if an event is available from the current state:

```soli
if (order.can("ship"))
    print("Order can be shipped")
end
```

### available_events()

Returns an array of events available from the current state:

```soli
let events = order.available_events();
// ["confirm", "cancel"] from pending state
```

## Instance Methods

### current_state()

Returns the current state as a string:

```soli
print(order.current_state());  // "pending"
```

### is(state)

Check if the state machine is in a specific state:

```soli
if (order.is("pending"))
    print("Order is waiting for confirmation")
end
```

### is_in([states])

Check if the state machine is in any of the given states:

```soli
if (order.is_in(["shipped", "delivered"]))
    print("Order is on its way or delivered")
end
```

### set(key, value)

Store custom data in the state machine context:

```soli
order.set("customer_id", 12345);
order.set("total", 99.99);
order.set("items", ["Product A", "Product B"]);
```

### get(key)

Retrieve custom data from the state machine context:

```soli
let customer_id = order.get("customer_id");
let total = order.get("total");
```

### history()

Get the state transition history:

```soli
let hist = order.history();
print(hist);
```

### last_transition()

Returns information about the last transition as a hash:

```soli
order.confirm();
let last = order.last_transition();
// {from => pending, to => confirmed, event => confirm}

print(last["from"]);  // "pending"
print(last["to"]);    // "confirmed"
print(last["event"]); // "confirm"
```

### can(event)

Checks if an event is available from the current state:

```soli
if (order.can("ship"))
    print("Order can be shipped")
end
# Returns: true or false
```

### available_events()

Returns an array of events available from the current state:

```soli
let events = order.available_events();
// ["confirm", "cancel"] from pending state
```

## Automatic Transition Methods

When you define transitions, Soli automatically creates methods for each event:

```soli
// From the transitions array above
order.confirm();   // Transitions from "pending" to "confirmed"
order.process();   // Transitions from "confirmed" to "processing"
order.ship();      // Transitions from "processing" to "shipped"
order.deliver();   // Transitions from "shipped" to "delivered"
order.cancel();    // Transitions from "pending" to "cancelled"
```

Invalid transitions are rejected:

```soli
order.ship();
// Error: Cannot transition 'ship' from state 'pending'. Valid states: processing
```

## Example: Order Processing Workflow

```soli
class OrderWorkflow
    fn create_order(items: Array, total: Float)
        let states = ["pending", "confirmed", "processing", "shipped", "delivered", "cancelled"]
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"},
            {"event": "ship", "from": "processing", "to": "shipped"},
            {"event": "deliver", "from": "shipped", "to": "delivered"},
            {"event": "cancel", "from": "pending", "to": "cancelled"}
        ]

        let order = create_state_machine("pending", states, transitions)
        order.set("items", items)
        order.set("total", total)
        order.set("created_at", clock())

        return order
    end

    fn process_order(order: Any)
        if (order.is("pending"))
            order.confirm()
        end

        if (order.is("confirmed"))
            order.process()
        end

        return order
    end
end
```

## Example: Payment State Machine

```soli
let payment_states = ["pending", "authorized", "captured", "failed", "refunded"];
let payment_transitions = [
    {"event": "authorize", "from": "pending", "to": "authorized"},
    {"event": "capture", "from": "authorized", "to": "captured"},
    {"event": "fail", "from": ["pending", "authorized"], "to": "failed"},
    {"event": "refund", "from": ["captured", "failed"], "to": "refunded"},
    {"event": "retry", "from": "failed", "to": "pending"}
];

let payment = create_state_machine("pending", payment_states, payment_transitions);
payment.set("amount", 99.99);
payment.set("currency", "USD");

payment.authorize();
print("Payment status: " + payment.current_state());  // "authorized"

payment.capture();
print("Payment status: " + payment.current_state());  // "captured"
```

## Database Persistence

State machines can be persisted to the database for long-running workflows:

```soli
// Save state to database
let state_data = {
    "machine_type": "Order",
    "machine_id": order.get("id"),
    "current_state": order.current_state(),
    "context": {
        "total": order.get("total"),
        "items": order.get("items")
    }
};
Order.update(order.get("id"), state_data);

// Load state from database
let saved_data = Order.find(order.get("id"));
let loaded_order = create_state_machine(
    saved_data["current_state"],
    ["pending", "confirmed", "processing", "shipped", "delivered"],
    [...]
);
loaded_order.set("total", saved_data["context"]["total"]);
loaded_order.set("items", saved_data["context"]["items"]);
```

## Best Practices

1. **Define clear states**: Use descriptive state names that reflect business states
2. **Validate transitions**: Let the state machine reject invalid transitions
3. **Use context for data**: Store related data in the state machine context
4. **One machine per workflow**: Use separate state machines for independent workflows
5. **Persist important states**: Save state changes for audit trails

## API Reference

### create_state_machine(initial_state, states, transitions)

Creates a new state machine instance.

**Parameters:**
- `initial_state` (String): Starting state
- `states` (Array): List of valid state names
- `transitions` (Array): Transition definitions with `event`, `from`, `to` keys

**Returns:** StateMachine instance

### Instance Methods

| Method | Description |
|--------|-------------|
| `current_state()` | Returns current state as string |
| `is(state)` | Returns true if in given state |
| `is_in([states])` | Returns true if in any of given states |
| `set(key, value)` | Stores data in context |
| `get(key)` | Retrieves data from context |
| `history()` | Returns transition history |
| `last_transition()` | Returns info about last transition |
| `can(event)` | Returns true if event is available |
| `available_events()` | Returns array of available events |

### Transition Methods

Automatic methods are created for each transition event defined in the transitions array.
