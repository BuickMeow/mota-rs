# frozen_string_literal: true
#
# RGSS .rxdata 读取库：Ruby Marshal + 动态 stub 类 + Table 解码。
#
#   require_relative "rxdata"
#   map = Rxdata.load("samples/魔塔样板7630·改/Data/Map004.rxdata")
#
# 注意：Marshal 里的 RPG::* 类在脚本外不存在，这里用空类 stub 顶住；
# Table 是 RGSS 的 C 扩展，按 _dump 格式手写 _load（dim,xs,ys,zs,size + 16bit）。

class Table
  attr_accessor :dim, :xs, :ys, :zs, :vals

  def self._load(dump)
    dim, xs, ys, zs, size = dump.unpack("L5")
    t = new
    t.dim = dim
    t.xs = xs
    t.ys = ys
    t.zs = zs
    t.vals = dump[20, size * 2].unpack("s<*")
    t
  end

  # Table[x, y, z]（RGSS 下标顺序）
  def [](x, y = 0, z = 0)
    vals[z * xs * ys + y * xs + x]
  end
end

module Rxdata
  module_function

  def load(path)
    data = File.binread(path)
    begin
      Marshal.load(data)
    rescue ArgumentError => e
      raise unless e.message =~ /undefined class\/module (.+)/
      define_stub(Regexp.last_match(1))
      retry
    end
  end

  # 把 "RPG::Event::Page::Condition" 这样的类名按层级补成空类
  def define_stub(name)
    mod = Object
    parts = name.split("::")
    parts.each_with_index do |part, i|
      if i == parts.length - 1
        mod.const_set(part, Class.new) unless mod.const_defined?(part)
      else
        mod.const_set(part, Module.new) unless mod.const_defined?(part)
        mod = mod.const_get(part)
      end
    end
  end

  # Marshal 字符串可能带非 UTF-8 尾巴，打印前洗一下
  def text(value)
    value.to_s.dup.force_encoding("UTF-8").scrub("?")
  end
end

module RPG; end
