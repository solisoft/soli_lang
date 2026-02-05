// Test SOAP functionality
// Run with: cargo run -- examples/soap_test.sol

// Test 1: Wrap XML body in SOAP envelope
print("=== Test 1: SOAP.wrap ===")
let body = "<GetWeather xmlns=\"http://example.com/weather\"><City>New York</City></GetWeather>"
let envelope = SOAP.wrap(body)
print("Wrapped envelope:")
print(envelope)

// Test 2: Parse XML response
print("\n=== Test 2: SOAP.parse ===")
let xml_response = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body><GetWeatherResponse xmlns=\"http://example.com/weather\"><Temperature>72</Temperature><Condition>Sunny</Condition><Humidity>45</Humidity></GetWeatherResponse></soap:Body></soap:Envelope>"

let parsed = SOAP.parse(xml_response)
print("Parsed XML:")
print(parsed)

// Access nested values
if parsed != null {
    if parsed["soap:Envelope"] != null {
        let envelope_data = parsed["soap:Envelope"]
        print("Envelope data:", envelope_data)
    }
}

// Test 3: XML escape
print("\n=== Test 3: SOAP.xml_escape ===")
let text = "Test with special chars: <>&\"'"
let escaped = SOAP.xml_escape(text)
print("Original:", text)
print("Escaped:", escaped)

print("\n=== All tests completed ===")
