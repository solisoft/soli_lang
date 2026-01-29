//! Example: Order State Machine
//!
//! This example demonstrates how to define and use a state machine
//! for order processing in an e-commerce application.

class Order extends StateMachine {
    static {
        // Define the initial state
        this.initial_state = "pending";

        // Define states
        this.state("pending", {
            initial: true,
            on_enter: fn() {
                print("Order created - status: pending");
            }
        });

        this.state("confirmed", {
            on_enter: fn() {
                print("Order confirmed by customer");
            },
            on_exit: fn() {
                print("Moving from confirmed to next state");
            }
        });

        this.state("processing", {
            on_enter: fn() {
                print("Order is being processed");
            }
        });

        this.state("shipped", {
            on_enter: fn() {
                let tracking = this.get("tracking_number");
                if (tracking != null) {
                    print("Order shipped with tracking: " + tracking);
                }
            }
        });

        this.state("delivered", {
            final: true,
            on_enter: fn() {
                print("Order delivered successfully!");
            }
        });

        this.state("cancelled", {
            final: true,
            on_enter: fn() {
                let reason = this.get("cancellation_reason");
                if (reason != null) {
                    print("Order cancelled: " + reason);
                }
            }
        });

        // Define transitions
        this.transition("confirm", from: "pending", to: "confirmed");
        this.transition("start_processing", from: "confirmed", to: "processing");
        this.transition("ship", from: "processing", to: "shipped", action: fn() {
            this.set("shipped_at", clock());
            print("Order has been shipped");
        });
        this.transition("deliver", from: "shipped", to: "delivered");

        // Conditional transition
        this.transition("cancel", from: ["pending", "confirmed"], to: "cancelled",
            guard: fn() {
                // Can only cancel if order is under $1000
                let total = this.get("total");
                return total == null || total < 1000;
            }
        );

        // Global transition callback
        this.on_transition(fn(from, to, event) {
            print("Transition: " + from + " -> " + to + " (event: " + event + ")");
        });
    }

    fn set_total(amount: Float) {
        this.set("total", amount);
    }

    fn set_tracking(tracking: String) {
        this.set("tracking_number", tracking);
    }

    fn cancel_order(reason: String) {
        this.set("cancellation_reason", reason);
        this.cancel();
    }
}

// Example usage
fn main() {
    print("=== Order State Machine Demo ===\n");

    // Create a new order
    let order = new Order();

    // Set some initial data
    order.set_total(150.0);
    order.set_tracking("TRK123456");

    print("Initial state: " + order.current_state());
    print("Is pending? " + str(order.is("pending")));
    print("");

    // Confirm the order
    order.confirm();
    print("State after confirm: " + order.current_state());
    print("");

    // Start processing
    order.start_processing();
    print("State after start_processing: " + order.current_state());
    print("");

    // Ship the order
    order.ship();
    print("State after ship: " + order.current_state());
    print("");

    // Deliver the order
    order.deliver();
    print("State after deliver: " + order.current_state());
    print("Is delivered? " + str(order.is("delivered")));
    print("");

    print("=== Order History ===");
    print("History: " + str(order.history()));
}

// Run the example
main();
