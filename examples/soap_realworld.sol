// Real-world SOAP example
// Calls a weather service (mock example)

// Build SOAP envelope (uses default SOAP 1.1 namespace)
let body = "<GetWeather xmlns=\"http://example.com/weather\"><City>London</City><Country>UK</Country></GetWeather>"

let envelope = SOAP.wrap(body)

// Escape for XML safety
let escaped_city = SOAP.xml_escape("New York <special>")

// Make SOAP call (using a mock URL for demonstration)
// In real usage, you would call:
// let result = await(SOAP.call("https://weather.example.com/service", "GetWeather", envelope))
// print(result["parsed"]["soap:Envelope"]["soap:Body"]["GetWeatherResponse"]["Temperature"])

print("SOAP Envelope ready:")
print(envelope)
print("")
print("XML escaped text:", escaped_city)
