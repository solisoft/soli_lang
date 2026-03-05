# Comprehensive String Methods Benchmark - Ruby Equivalent
# Tests only methods that work in Soli for fair comparison

$iterations = 10000
$test_string = "Hello World Test String 123"
$test_string_long = "The quick brown fox jumps over the lazy dog. This is a longer test string for benchmarking string operations in Soli. Testing various methods!"
$patterns_test = "hello world hello world hello"

# Test upcase
def bench_upcase
  i = 0
  while i < $iterations
    _ = $test_string.upcase
    i += 1
  end
end

# Test downcase
def bench_downcase
  i = 0
  while i < $iterations
    _ = $test_string.downcase
    i += 1
  end
end

# Test capitalize
def bench_capitalize
  i = 0
  while i < $iterations
    _ = $test_string.capitalize
    i += 1
  end
end

# Test swapcase
def bench_swapcase
  i = 0
  while i < $iterations
    _ = $test_string.swapcase
    i += 1
  end
end

# Test strip (trim)
def bench_trim
  i = 0
  s = "   " + $test_string + "   "
  while i < $iterations
    _ = s.strip
    i += 1
  end
end

# Test lstrip
def bench_lstrip
  i = 0
  s = "   " + $test_string
  while i < $iterations
    _ = s.lstrip
    i += 1
  end
end

# Test rstrip
def bench_rstrip
  i = 0
  s = $test_string + "   "
  while i < $iterations
    _ = s.rstrip
    i += 1
  end
end

# Test chomp
def bench_chomp
  i = 0
  s = $test_string + "\n"
  while i < $iterations
    _ = s.chomp
    i += 1
  end
end

# Test replace (Ruby uses gsub for replace)
def bench_replace
  i = 0
  while i < $iterations
    _ = $test_string.gsub("World", "Soli")
    i += 1
  end
end

# Test include? (contains)
def bench_contains
  i = 0
  while i < $iterations
    _ = $test_string.include?("World")
    i += 1
  end
end

# Test start_with?
def bench_starts_with
  i = 0
  while i < $iterations
    _ = $test_string.start_with?("Hello")
    i += 1
  end
end

# Test end_with?
def bench_ends_with
  i = 0
  while i < $iterations
    _ = $test_string.end_with?("123")
    i += 1
  end
end

# Test length (len)
def bench_len
  i = 0
  while i < $iterations
    _ = $test_string.length
    i += 1
  end
end

# Test squeeze
def bench_squeeze
  i = 0
  while i < $iterations
    _ = "aaaabbbbcccc".squeeze
    i += 1
  end
end

# Test gsub
def bench_gsub
  i = 0
  while i < $iterations
    _ = $test_string.gsub("o", "0")
    i += 1
  end
end

# Test tr
def bench_tr
  i = 0
  while i < $iterations
    _ = $test_string.tr("aeiou", "AEIOU")
    i += 1
  end
end

# Test center
def bench_center
  i = 0
  while i < $iterations
    _ = "hi".center(10)
    i += 1
  end
end

# Test ljust
def bench_ljust
  i = 0
  while i < $iterations
    _ = "hi".ljust(10)
    i += 1
  end
end

# Test rjust
def bench_rjust
  i = 0
  while i < $iterations
    _ = "hi".rjust(10)
    i += 1
  end
end

# Test ljust (lpad equivalent in Ruby)
def bench_lpad
  i = 0
  while i < $iterations
    _ = "hi".ljust(10)
    i += 1
  end
end

# Test rjust (rpad equivalent in Ruby)
def bench_rpad
  i = 0
  while i < $iterations
    _ = "hi".rjust(10)
    i += 1
  end
end

# Test chars
def bench_chars
  i = 0
  while i < $iterations
    _ = $test_string.chars
    i += 1
  end
end

# Test bytes
def bench_bytes
  i = 0
  while i < $iterations
    _ = $test_string.bytes
    i += 1
  end
end

# Test lines
def bench_lines
  i = 0
  s = "line1\nline2\nline3\nline4\nline5"
  while i < $iterations
    _ = s.lines
    i += 1
  end
end

# Test reverse
def bench_reverse
  i = 0
  while i < $iterations
    _ = $test_string.reverse
    i += 1
  end
end

# Test bytesize
def bench_bytesize
  i = 0
  while i < $iterations
    _ = $test_string.bytesize
    i += 1
  end
end

# Test hex
def bench_hex
  i = 0
  while i < $iterations
    _ = "ff".hex
    i += 1
  end
end

# Test oct
def bench_oct
  i = 0
  while i < $iterations
    _ = "77".oct
    i += 1
  end
end

# Test empty?
def bench_empty
  i = 0
  while i < $iterations
    _ = $test_string.empty?
    i += 1
  end
end

# Test include? (already tested as contains)
def bench_include
  i = 0
  while i < $iterations
    _ = $test_string.include?("World")
    i += 1
  end
end

# Run all benchmarks
bench_upcase
bench_downcase
bench_capitalize
bench_swapcase
bench_trim
bench_lstrip
bench_rstrip
bench_chomp
bench_replace
bench_contains
bench_starts_with
bench_ends_with
bench_len
bench_squeeze
bench_gsub
bench_tr
bench_center
bench_ljust
bench_rjust
bench_lpad
bench_rpad
bench_chars
bench_bytes
bench_lines
bench_reverse
bench_bytesize
bench_hex
bench_oct
bench_empty
bench_include
