# Round trip: the message goes back to the sender only.
class EchoChannel < ApplicationCable::Channel
  def subscribed
    stream_from "echo_#{SecureRandom.hex(8)}"
  end

  def echo(data)
    transmit(data["body"])
  end
end
