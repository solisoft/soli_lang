
export class StateMachine {
    states: Array;
    transitions: Array;
    _current_state: String;
    _history: Array;
    _last_transition: Hash;
    valid_events: Hash;
    context: Hash;

    new(initial_state: String, states: Array, transitions: Array) {
        this.states = states;
        this.transitions = transitions;
        this._current_state = initial_state;
        this._history = [];
        this._last_transition = null;
        this.context = {};
        
        // Build valid events map
        this.valid_events = {};
        let t_idx = 0;
        while t_idx < len(transitions) {
            let transition = transitions[t_idx];
            let event = transition["event"];
            let sources = transition["from"];
            if type(sources) == "String" {
                sources = [sources];
            }
            if !has_key(this.valid_events, event) {
                this.valid_events[event] = sources;
            } else {
                let current = this.valid_events[event];
                let new_sources = [];
                let s_idx = 0;
                while s_idx < len(sources) {
                    let s = sources[s_idx];
                    // Use Array.find() to check if source already exists
                    let existing = current.find(fn(x) x == s);
                    if existing == null {
                        new_sources = [...new_sources, s];
                    }
                    s_idx = s_idx + 1;
                }
                if len(new_sources) > 0 {
                    this.valid_events[event] = [...current, ...new_sources];
                }
            }
            t_idx = t_idx + 1;
        }
    }

    fn current_state() -> String {
        return this._current_state;
    }

    fn is(state: String) -> Bool {
        return this._current_state == state;
    }

    fn is_in(states: Array) -> Bool {
        // Use Array.find() to check if current state is in list
        return states.find(fn(x) x == this._current_state) != null;
    }

    fn can(event: String) -> Bool {
        if !has_key(this.valid_events, event) {
            return false;
        }
        // Use Array.find() to check if current state can trigger event
        return this.valid_events[event].find(fn(x) x == this._current_state) != null;
    }

    fn available_events() -> Array {
        let all_events = keys(this.valid_events);
        let result = [];
        let e_idx = 0;
        while e_idx < len(all_events) {
            let event = all_events[e_idx];
            // Use Array.find() to check if event is available from current state
            if this.valid_events[event].find(fn(x) x == this._current_state) != null {
                result = [...result, event];
            }
            e_idx = e_idx + 1;
        }
        return result;
    }

    fn transition(event: String) -> Hash {
        let idx = 0;
        while idx < len(this.transitions) {
            let transition = this.transitions[idx];
            if transition["event"] == event {
                let sources = transition["from"];
                let is_valid = false;
                if type(sources) == "String" {
                    if sources == this._current_state {
                        is_valid = true;
                    }
                } else {
                    // Use Array.find() to check if current state is a valid source
                    if sources.find(fn(x) x == this._current_state) != null {
                        is_valid = true;
                    }
                }
                if is_valid {
                    let from_state = this._current_state;
                    let to_state = transition["to"];
                    this._current_state = to_state;
                    this._history = [...this._history, {
                        "from": from_state,
                        "to": to_state,
                        "event": event
                    }];
                    this._last_transition = {
                        "from": from_state,
                        "to": to_state,
                        "event": event
                    };
                    return {
                        "success": true,
                        "from": from_state,
                        "to": to_state,
                        "event": event
                    };
                }
            }
            idx = idx + 1;
        }
        
        return {
            "success": false,
            "error": "invalid_transition",
            "reason": "Cannot transition '" + event + "' from state '" + this._current_state + "'"
        };
    }

    fn set(key: String, value: Any) {
        this.context[key] = value;
    }

    fn get(key: String) -> Any {
        if has_key(this.context, key) {
            return this.context[key];
        }
        return null;
    }

    fn history() -> Array {
        return this._history;
    }

    fn last_transition() -> Hash {
        return this._last_transition;
    }
}

export fn create_state_machine(initial_state: String, states: Array, transitions: Array) -> StateMachine {
    return new StateMachine(initial_state, states, transitions);
}

export class StateMachineBuilder {
    initial_state: String;
    states: Array;
    transitions: Array;

    new() {
        this.initial_state = "";
        this.states = [];
        this.transitions = [];
    }

    fn initial(state: String) -> StateMachineBuilder {
        this.initial_state = state;
        return this;
    }

    fn states_list(states: Array) -> StateMachineBuilder {
        this.states = states;
        return this;
    }

    fn transition(event: String, from_state: Any, to: String) -> StateMachineBuilder {
        let sources = from_state;
        if type(from_state) == "String" {
            sources = [from_state];
        }
        this.transitions = [...this.transitions, {
            "event": event,
            "from": sources,
            "to": to
        }];
        return this;
    }

    fn build() -> StateMachine {
        return create_state_machine(this.initial_state, this.states, this.transitions);
    }
}

export fn state_machine() -> StateMachineBuilder {
    return new StateMachineBuilder();
}
