// StateMachine Controller - REST API for State Machine management

import "../../stdlib/state_machine.sl";

// In-memory store for demo (use SolidB/DB in production)
let state_machines: Hash = {};

// Create a new state machine
fn create(req: Any) -> Any {
    let data = req["json"];
    
    // Validate required fields
    if data["initial_state"] == null {
        return {
            "status": 422,
            "body": json_stringify({
                "success": false,
                "error": "initial_state is required"
            })
        };
    }
    
    if data["states"] == null {
        return {
            "status": 422,
            "body": json_stringify({
                "success": false,
                "error": "states array is required"
            })
        };
    }
    
    if data["transitions"] == null {
        return {
            "status": 422,
            "body": json_stringify({
                "success": false,
                "error": "transitions array is required"
            })
        };
    }
    
    // Generate unique ID
    let id = "sm_" + str(clock()) + "_" + str(rand() * 10000);
    
    // Create state machine
    let sm = create_state_machine(
        data["initial_state"],
        data["states"],
        data["transitions"]
    );
    
    // Store in memory
    state_machines[id] = sm;
    
    return {
        "status": 201,
        "body": json_stringify({
            "success": true,
            "id": id,
            "state": sm.current_state(),
            "available_events": sm.available_events()
        })
    };
}

// Get state machine by ID
fn get(req: Any) -> Any {
    let id = req["params"]["id"];
    
    if !has_key(state_machines, id) {
        return {
            "status": 404,
            "body": json_stringify({
                "success": false,
                "error": "State machine not found"
            })
        };
    }
    
    let sm = state_machines[id];
    
    return {
        "status": 200,
        "body": json_stringify({
            "success": true,
            "id": id,
            "state": sm.current_state(),
            "states": sm.states,
            "available_events": sm.available_events(),
            "can": sm.can,
            "history": sm.history(),
            "last_transition": sm.last_transition()
        })
    };
}

// List all state machines
fn list(req: Any) -> Any {
    let list = [];
    for id, sm in state_machines {
        list = [...list, {
            "id": id,
            "current_state": sm.current_state(),
            "available_events": sm.available_events()
        }];
    }
    
    return {
        "status": 200,
        "body": json_stringify({
            "success": true,
            "count": len(list),
            "state_machines": list
        })
    };
}

// Perform a transition
fn transition(req: Any) -> Any {
    let id = req["params"]["id"];
    let data = req["json"];
    
    if !has_key(state_machines, id) {
        return {
            "status": 404,
            "body": json_stringify({
                "success": false,
                "error": "State machine not found"
            })
        };
    }
    
    let event = data["event"];
    if event == null {
        return {
            "status": 422,
            "body": json_stringify({
                "success": false,
                "error": "event is required"
            })
        };
    }
    
    let sm = state_machines[id];
    
    // Check if event is available
    if !sm.can(event) {
        return {
            "status": 400,
            "body": json_stringify({
                "success": false,
                "error": "Event '" + event + "' is not available from current state '" + sm.current_state() + "'",
                "current_state": sm.current_state(),
                "available_events": sm.available_events()
            })
        };
    }
    
    // Perform transition
    let result = sm.transition(event);
    
    return {
        "status": 200,
        "body": json_stringify({
            "success": result["success"],
            "from": result["from"],
            "to": result["to"],
            "event": result["event"],
            "current_state": sm.current_state(),
            "available_events": sm.available_events()
        })
    };
}

// Set context value
fn set_context(req: Any) -> Any {
    let id = req["params"]["id"];
    let data = req["json"];
    
    if !has_key(state_machines, id) {
        return {
            "status": 404,
            "body": json_stringify({
                "success": false,
                "error": "State machine not found"
            })
        };
    }
    
    let key = data["key"];
    if key == null {
        return {
            "status": 422,
            "body": json_stringify({
                "success": false,
                "error": "key is required"
            })
        };
    }
    
    let sm = state_machines[id];
    sm.set(key, data["value"]);
    
    return {
        "status": 200,
        "body": json_stringify({
            "success": true,
            "key": key,
            "value": data["value"]
        })
    };
}

// Get context value
fn get_context(req: Any) -> Any {
    let id = req["params"]["id"];
    let key = req["params"]["key"];
    
    if !has_key(state_machines, id) {
        return {
            "status": 404,
            "body": json_stringify({
                "success": false,
                "error": "State machine not found"
            })
        };
    }
    
    let sm = state_machines[id];
    let value = sm.get(key);
    
    return {
        "status": 200,
        "body": json_stringify({
            "success": true,
            "key": key,
            "value": value
        })
    };
}

// Delete state machine
fn delete(req: Any) -> Any {
    let id = req["params"]["id"];
    
    if !has_key(state_machines, id) {
        return {
            "status": 404,
            "body": json_stringify({
                "success": false,
                "error": "State machine not found"
            })
        };
    }
    
    state_machines[id] = null;
    
    return {
        "status": 200,
        "body": json_stringify({
            "success": true,
            "message": "State machine deleted"
        })
    };
}

// Demo page
fn demo(req: Any) -> Any {
    return render("state_machines/demo.html", {
        "title": "State Machine Demo"
    });
}
