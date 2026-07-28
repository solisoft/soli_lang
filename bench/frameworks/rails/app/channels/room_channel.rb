# Fan-out: every subscriber of the room receives, across all Puma workers via
# the redis adapter.
class RoomChannel < ApplicationCable::Channel
  def subscribed
    stream_from "room"
  end

  def speak(data)
    ActionCable.server.broadcast("room", data["body"])
  end
end
