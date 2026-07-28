Post.delete_all
(1..50).each { |i| Post.create!(id: i, title: "Post title #{i}", views: i * 7) }
puts "seeded: #{Post.count}"
