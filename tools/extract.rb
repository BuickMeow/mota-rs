#!/usr/bin/env ruby
# frozen_string_literal: true
#
# 7630·改 数据查看/维护工具。
#
# 用法：
#   ruby tools/extract.rb dump <rxdata|Map004> [事件id...]   地图事件 / 公共事件列表
#   ruby tools/extract.rb ce <id>                           单个公共事件的指令
#   ruby tools/extract.rb patch-floors                       把 .rxdata 可推导字段回填 data/floors
#
# patch-floors 只回填能由地图数据推导的字段（保留手工命名/贴图/id）：
#   - NPC 生命周期（CE13 → once；switch32 置位 → respawn_on_reenter）
#   - 行走/停止时动画（walk_anime / step_anime）
#   - 复活怪（有 revive 特技）的 respawn_on_reenter
#   - 开始地图（m02）的自动剧情 intro（台词 + 播完传送坐标）
#
# 为什么不做"全量重生成"：怪物/物品/门的 id 与贴图是早期提取时手工命名/挑选的，
# 没有可推导的映射；这里只维护能自动推导的部分，避免把 id 打乱。

require "json"
require_relative "rxdata"

ROOT = File.expand_path("..", __dir__)
SAMPLE_DATA = File.join(ROOT, "samples/魔塔样板7630·改/Data")
FLOORS_DIR = File.join(ROOT, "data/floors")
ENEMIES_DIR = File.join(ROOT, "data/enemies")

def load_map(path)
  Rxdata.load(path)
end

def load_common_events(path = File.join(SAMPLE_DATA, "CommonEvents.rxdata"))
  Rxdata.load(path)
end

def page_conditions(page)
  cond = page.instance_variable_get(:@condition)
  list = []
  if cond.instance_variable_get(:@switch1_valid)
    list << "sw#{cond.instance_variable_get(:@switch1_id)}"
  end
  if cond.instance_variable_get(:@switch2_valid)
    list << "sw#{cond.instance_variable_get(:@switch2_id)}"
  end
  if cond.instance_variable_get(:@variable_valid)
    list << "v#{cond.instance_variable_get(:@variable_id)}>=#{cond.instance_variable_get(:@variable_value)}"
  end
  if cond.instance_variable_get(:@self_switch_valid)
    list << "ss#{Rxdata.text(cond.instance_variable_get(:@self_switch_ch))}"
  end
  list
end

def command_summary(c)
  code = c.instance_variable_get(:@code)
  p = c.instance_variable_get(:@parameters) || []
  case code
  when 101, 401 then "文章 #{Rxdata.text(p[0])}"
  when 108, 408 then "注释 #{Rxdata.text(p[0])}"
  when 121 then "开关 #{p[0]} = #{p[2].to_i.zero? ? 'ON' : 'OFF'}"
  when 122 then "变量 #{p[0]} op#{p[2]} t#{p[3]} #{p[4].inspect}"
  when 123 then "独立开关 #{Rxdata.text(p[0])} = #{p[1].to_i.zero? ? 'ON' : 'OFF'}"
  when 111 then "条件分支 #{p.inspect}"
  when 117 then "调用公共事件 #{p[0]}"
  when 201 then "场所移动 map#{p[0]} (#{p[1]},#{p[2]})"
  when 355, 655 then "脚本 #{Rxdata.text(p[0])}"
  else "c#{code} #{p.inspect}"
  end
end

# ---------------------------------------------------------------------------
# dump：地图事件 / 公共事件
# ---------------------------------------------------------------------------
def cmd_dump(argv)
  path = argv.shift or abort("用法: extract.rb dump <rxdata> [事件id...]")
  path = File.join(SAMPLE_DATA, path) unless File.exist?(path)
  path += ".rxdata" unless File.exist?(path)
  ids = argv.map(&:to_i)
  obj = Rxdata.load(path)

  if obj.instance_variable_get(:@events)
    events = obj.instance_variable_get(:@events)
    events.keys.sort.each do |id|
      next unless ids.empty? || ids.include?(id)
      e = events[id]
      name = Rxdata.text(e.instance_variable_get(:@name))
      puts "== event #{id} #{name} (#{e.instance_variable_get(:@x)},#{e.instance_variable_get(:@y)})"
      (e.instance_variable_get(:@pages) || []).each_with_index do |pg, i|
        puts "   page#{i} trigger=#{pg.instance_variable_get(:@trigger)} cond=#{page_conditions(pg).inspect}"
        (pg.instance_variable_get(:@list) || []).each do |c|
          puts "      #{command_summary(c)}"
        end
      end
    end
  else
    obj.each do |ce|
      next if ce.nil?
      id = ce.instance_variable_get(:@id)
      next unless ids.empty? || ids.include?(id)
      dump_common_event(ce)
    end
  end
end

def dump_common_event(ce)
  puts "== CE##{ce.instance_variable_get(:@id)} #{Rxdata.text(ce.instance_variable_get(:@name))} " \
       "trigger=#{ce.instance_variable_get(:@trigger)}"
  (ce.instance_variable_get(:@list) || []).each do |c|
    indent = "  " * c.instance_variable_get(:@indent)
    puts "   #{indent}#{command_summary(c)}"
  end
end

def cmd_ce(argv)
  id = (argv.shift or abort("用法: extract.rb ce <id>")).to_i
  ce = load_common_events.find { |e| e && e.instance_variable_get(:@id) == id }
  abort("CE #{id} 不存在") unless ce
  dump_common_event(ce)
end

# ---------------------------------------------------------------------------
# patch-floors：从 .rxdata 回填可推导字段
# ---------------------------------------------------------------------------
def cmd_patch_floors(_argv)
  (1..8).each do |n|
    stem = format("m%02d", n)
    json_path = File.join(FLOORS_DIR, "#{stem}.json")
    map_path = File.join(SAMPLE_DATA, format("Map%03d.rxdata", n))
    next unless File.exist?(json_path) && File.exist?(map_path)

    events = load_map(map_path).instance_variable_get(:@events)
    obj = JSON.parse(File.read(json_path))
    changed = 0

    (obj["instances"] || []).each do |inst|
      m = inst["id"].match(/_e(\d+)$/)
      next unless m
      event = events[m[1].to_i]
      next unless event

      page = (event.instance_variable_get(:@pages) || []).first
      next unless page

      changed += patch_anim(inst, page)
      changed += patch_lifecycle(inst, page)
    end

    if stem == "m02"
      intro = map_intro(events)
      if intro && obj["intro"] != intro
        obj["intro"] = intro
        changed += 1
      end
    end

    next if changed.zero?

    File.write(json_path, JSON.pretty_generate(obj) + "\n")
    puts "#{stem}: 更新 #{changed} 处"
  end
end

def patch_anim(inst, page)
  n = 0
  { "walk_anime" => :@walk_anime, "step_anime" => :@step_anime }.each do |key, ivar|
    want = page.instance_variable_get(ivar) ? true : false
    if want
      unless inst[key]
        inst[key] = true
        n += 1
      end
    elsif inst.key?(key)
      inst.delete(key)
      n += 1
    end
  end
  n
end

# CE13（事件结束处理）：默认（switch32 OFF）改名 dead + 独立开关 A → once；
# switch32 置位（重生怪）走 erase → respawn_on_reenter。
def patch_lifecycle(inst, page)
  kind = inst.dig("template", "kind")
  return 0 unless %w[npc monster].include?(kind)

  want =
    if kind == "npc"
      npc_lifecycle(page)
    else
      monster_lifecycle(inst.dig("template", "monster_id"))
    end
  return 0 if inst["lifecycle"] == want

  inst["lifecycle"] = want
  1
end

def npc_lifecycle(page)
  switch32 = false
  (page.instance_variable_get(:@list) || []).each do |c|
    next unless c.instance_variable_get(:@indent).zero?
    code = c.instance_variable_get(:@code)
    p = c.instance_variable_get(:@parameters) || []
    if code == 121 && p[0].to_i <= 32 && p[1].to_i >= 32
      switch32 = p[2].to_i.zero?
    elsif code == 123 && p[0] == "A" && p[1].to_i.zero?
      return "once"
    elsif code == 117 && p[0] == 13
      return switch32 ? "respawn_on_reenter" : "once"
    end
  end
  nil
end

def monster_lifecycle(monster_id)
  path = File.join(ENEMIES_DIR, "#{monster_id}.json")
  return nil unless File.exist?(path)
  enemy = JSON.parse(File.read(path))
  skills = enemy["skills"] || []
  return "respawn_on_reenter" if skills.any? { |s| s.split(":").first == "revive" }

  nil
end

# 开始地图：取自动执行（trigger=3）事件的台词，和后续 201 传送坐标。
# 7630 改过 command_201：params[0]==0 时 map=params[1], x=params[2], y=params[3]。
def map_intro(events)
  events.keys.sort.each do |id|
    event = events[id]
    pages = event.instance_variable_get(:@pages) || []
    first = pages.first
    next unless first && first.instance_variable_get(:@trigger) == 3

    lines = []
    (first.instance_variable_get(:@list) || []).each do |c|
      code = c.instance_variable_get(:@code)
      lines << Rxdata.text((c.instance_variable_get(:@parameters) || [])[0]) if [101, 401].include?(code)
    end
    target = nil
    pages.each do |pg|
      (pg.instance_variable_get(:@list) || []).each do |c|
        next unless c.instance_variable_get(:@code) == 201
        p = c.instance_variable_get(:@parameters) || []
        target = { floor: format("m%02d", p[1].to_i), x: p[2].to_i, y: p[3].to_i } if p[0].to_i.zero?
      end
    end

    intro = { "lines" => lines }
    if target
      intro["to_floor"] = target[:floor]
      intro["to_x"] = target[:x]
      intro["to_y"] = target[:y]
    end
    return intro
  end
  nil
end

case ARGV[0]
when "dump" then cmd_dump(ARGV[1..])
when "ce" then cmd_ce(ARGV[1..])
when "patch-floors" then cmd_patch_floors(ARGV[1..])
else
  puts File.read(__FILE__)[/^#\s.*?(?=^require)/m] || "用法见文件头注释"
end
