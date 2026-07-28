# WebSocket workloads. `echo` measures the per-message round trip; `room`
# measures fan-out, where one client's message reaches every connection.
def echo(event: Any) -> Any {
  return { "send": event["message"] } if event["type"] == "message"
  return {}
}

def room(event: Any) -> Any {
  return { "broadcast": event["message"] } if event["type"] == "message"
  return {}
}
