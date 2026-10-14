# frozen_string_literal: true

module Example
  class Thing
    def initialize(a, b)
      @a = a
      @b = b
    end

    def sum
      @a + @b
    end

    def product
      @a * @b
    end

    def describe
      "#{@a} & #{@b} <ok>"
    end
  end
end

puts Example::Thing.new(1, 2).sum
puts Example::Thing.new(1, 2).product
puts Example::Thing.new(1, 2).describe
