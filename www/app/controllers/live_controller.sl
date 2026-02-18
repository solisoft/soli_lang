// LiveView controller - handles Live View events
//
// Live View handlers receive an event hash with:
// - event: The event name (e.g., "increment", "decrement")
// - params: Parameters sent with the event
// - state: The current component state
//
// Handlers should return the new state as a hash.

// Counter component handler
fn counter(event_data)
    let event = event_data["event"]
    let state = event_data["state"]
    let count = state["count"]

    if (count == null)
        count = 0
    end

    if (event == "increment")
        return {
            "count": count + 1
        }
    end

    if (event == "decrement")
        return {
            "count": count - 1
        }
    end

    // Return unchanged state for unknown events
    state
end

// Metrics component handler - Binary Clock
fn metrics(event_data)
    let event = event_data["event"]
    let state = event_data["state"]

    if (event == "tick")
        // Get current time using DateTime class
        let now = DateTime.utc()
        let h = now.hour()
        let m = now.minute()
        let s = now.second()
        let ms = now.millisecond()

        // Format strings with leading zeros
        let hours_str = "" + h
        let minutes_str = "" + m
        let seconds_str = "" + s

        if (h < 10)
            hours_str = "0" + h
        end
        if (m < 10)
            minutes_str = "0" + m
        end
        if (s < 10)
            seconds_str = "0" + s
        end

        let milliseconds_str = "" + ms
        if (ms < 100)
            if (ms < 10)
                milliseconds_str = "00" + ms
            else
                milliseconds_str = "0" + ms
            end
        end

        // Binary clock bits (pre-computed for template)
        // Hours: 5 bits (0-23)
        let h4 = 0
        let h3 = 0
        let h2 = 0
        let h1 = 0
        let h0 = 0

        let hv = h
        if (hv >= 16)  h4 = 1; hv = hv - 16 end
        if (hv >= 8)  h3 = 1; hv = hv - 8 end
        if (hv >= 4)  h2 = 1; hv = hv - 4 end
        if (hv >= 2)  h1 = 1; hv = hv - 2 end
        if (hv >= 1)  h0 = 1 end

        // Minutes: 6 bits (0-59)
        let m5 = 0
        let m4 = 0
        let m3 = 0
        let m2 = 0
        let m1 = 0
        let m0 = 0

        let mv = m
        if (mv >= 32)  m5 = 1; mv = mv - 32 end
        if (mv >= 16)  m4 = 1; mv = mv - 16 end
        if (mv >= 8)  m3 = 1; mv = mv - 8 end
        if (mv >= 4)  m2 = 1; mv = mv - 4 end
        if (mv >= 2)  m1 = 1; mv = mv - 2 end
        if (mv >= 1)  m0 = 1 end

        // Seconds: 6 bits (0-59)
        let s5 = 0
        let s4 = 0
        let s3 = 0
        let s2 = 0
        let s1 = 0
        let s0 = 0

        let sv = s
        if (sv >= 32)  s5 = 1; sv = sv - 32 end
        if (sv >= 16)  s4 = 1; sv = sv - 16 end
        if (sv >= 8)  s3 = 1; sv = sv - 8 end
        if (sv >= 4)  s2 = 1; sv = sv - 4 end
        if (sv >= 2)  s1 = 1; sv = sv - 2 end
        if (sv >= 1)  s0 = 1 end

        // Milliseconds: 10 bits (0-999)
        let ms9 = 0
        let ms8 = 0
        let ms7 = 0
        let ms6 = 0
        let ms5 = 0
        let ms4 = 0
        let ms3 = 0
        let ms2 = 0
        let ms1 = 0
        let ms0 = 0

        let msv = ms
        if (msv >= 512)  ms9 = 1; msv = msv - 512 end
        if (msv >= 256)  ms8 = 1; msv = msv - 256 end
        if (msv >= 128)  ms7 = 1; msv = msv - 128 end
        if (msv >= 64)  ms6 = 1; msv = msv - 64 end
        if (msv >= 32)  ms5 = 1; msv = msv - 32 end
        if (msv >= 16)  ms4 = 1; msv = msv - 16 end
        if (msv >= 8)  ms3 = 1; msv = msv - 8 end
        if (msv >= 4)  ms2 = 1; msv = msv - 4 end
        if (msv >= 2)  ms1 = 1; msv = msv - 2 end
        if (msv >= 1)  ms0 = 1 end

        return {
            "hours_str": hours_str,
            "minutes_str": minutes_str,
            "seconds_str": seconds_str,
            "milliseconds": ms,
            "milliseconds_str": milliseconds_str,
            "h4": h4, "h3": h3, "h2": h2, "h1": h1, "h0": h0,
            "m5": m5, "m4": m4, "m3": m3, "m2": m2, "m1": m1, "m0": m0,
            "s5": s5, "s4": s4, "s3": s3, "s2": s2, "s1": s1, "s0": s0,
            "ms9": ms9, "ms8": ms8, "ms7": ms7, "ms6": ms6, "ms5": ms5,
            "ms4": ms4, "ms3": ms3, "ms2": ms2, "ms1": ms1, "ms0": ms0
        }
    end

    null
end
