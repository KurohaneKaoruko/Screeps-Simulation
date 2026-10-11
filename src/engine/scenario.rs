//! 场景定义与加载: JSON → World 初始状态。

use crate::engine::constants;
use crate::engine::objects::{self, BodyPart};
use crate::engine::terrain::{self, Terrain};
use crate::engine::world::{alloc_id, prime_world, World};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum TerrainSpec {
    /// 全平原 (确定性, 行为验证用)
    #[serde(rename = "flat")]
    Flat,
    /// 种子噪声生成 (近似官方地形风格, 非复刻)
    #[serde(rename = "generated")]
    Generated { seed: Option<u64> },
    /// 50 行显式地形字符 ('.'/plain=0, '#'/wall=1, '~'/swamp=2) 或 2500 字符数字串
    #[serde(rename = "rows")]
    Rows { rows: Vec<String> },
    /// 2500 字符数字串 (可用 getRawBuffer 导出后转写)
    #[serde(rename = "raw")]
    Raw { data: String },
}

#[derive(Debug, Clone)]
pub enum SourcesSpec {
    /// 自动放置: count (默认 2) / energy (默认 3000)
    Auto { count: Option<u32>, energy: Option<u32> },
    /// 显式位置
    Explicit { positions: Vec<[u8; 2]> },
}

/// 含 positions 键 → 显式位置, 否则视为自动放置
/// (不能使用 untagged: Auto 全 Option 字段会吞掉任何未知形态)
impl<'de> serde::Deserialize<'de> for SourcesSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SourcesVisitor;
        impl<'de> serde::de::Visitor<'de> for SourcesVisitor {
            type Value = SourcesSpec;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("sources 配置对象")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<SourcesSpec, A::Error> {
                let mut count = None;
                let mut energy = None;
                let mut positions = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "count" => count = map.next_value()?,
                        "energy" => energy = map.next_value()?,
                        "positions" => positions = Some(map.next_value()?),
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                if let Some(positions) = positions {
                    Ok(SourcesSpec::Explicit { positions })
                } else {
                    Ok(SourcesSpec::Auto { count, energy })
                }
            }
        }
        deserializer.deserialize_map(SourcesVisitor)
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct ControllerSpec {
    pub level: u8,
    pub pos: Option<[u8; 2]>,
    pub progress: Option<u32>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct StructureSpec {
    #[serde(rename = "type")]
    pub structure_type: String,
    pub pos: [u8; 2],
    pub energy: Option<u32>,
    pub hits: Option<u32>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct CreepSpec {
    pub name: String,
    pub body: Vec<String>,
    pub pos: [u8; 2],
    pub ttl: Option<u64>,
    #[serde(default)]
    pub store: BTreeMap<String, u32>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RoomSpec {
    pub name: String,
    #[serde(default)]
    pub terrain: Option<TerrainSpec>,
    #[serde(default)]
    pub sources: Option<SourcesSpec>,
    #[serde(default)]
    pub controller: Option<ControllerSpec>,
    #[serde(default)]
    pub structures: Vec<StructureSpec>,
    #[serde(default)]
    pub creeps: Vec<CreepSpec>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct CpuSpec {
    pub limit: Option<f64>,
    pub bucket: Option<f64>,
    pub tick_limit: Option<f64>,
    /// 超过 tickLimit 时是否中断当 tick (默认仅计量)
    #[serde(default)]
    pub enforce: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum AssertSpec {
    #[serde(rename = "rclAtLeast")]
    RclAtLeast { room: Option<String>, value: u8 },
    #[serde(rename = "rclReachedBy")]
    RclReachedBy { value: u8, tick: u64 },
    #[serde(rename = "creepCountAtLeast")]
    CreepCountAtLeast { value: usize },
    #[serde(rename = "cpuAvgBelow")]
    CpuAvgBelow { value: f64 },
    #[serde(rename = "cpuP95Below")]
    CpuP95Below { value: f64 },
    #[serde(rename = "noErrors")]
    NoErrors,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Scenario {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// 默认运行 tick 数 (CLI --ticks 覆盖)
    pub ticks: Option<u64>,
    #[serde(default)]
    pub cpu: Option<CpuSpec>,
    /// 初始 Memory JSON 字符串
    #[serde(default)]
    pub memory: Option<String>,
    /// 调试: 所有房间始终可见
    #[serde(default)]
    pub always_visible: bool,
    /// 用确定性 Math.random (复现实验)
    #[serde(default = "default_true")]
    pub deterministic_random: bool,
    /// 每 N tick 重建 isolate 做一次"全局重置"(官方由 isolate 堆占用触发, 这里提供确定性开关)。
    /// 全局变量丢失、Memory 与内存段保留。
    #[serde(default, rename = "globalResetEvery")]
    pub global_reset_every: Option<u64>,
    #[serde(default)]
    pub rooms: Vec<RoomSpec>,
    #[serde(default)]
    pub asserts: Vec<AssertSpec>,
}

fn default_true() -> bool {
    true
}

impl Scenario {
    /// 解析 + 校验场景 (校验在构建世界之前, 错误信息带房间/字段定位)
    pub fn from_json(text: &str) -> Result<Scenario, String> {
        let scenario: Scenario =
            serde_json::from_str(text).map_err(|e| format!("场景解析失败: {e}"))?;
        scenario.validate()?;
        Ok(scenario)
    }

    /// 场景校验: 房名/坐标/等级/结构类型/部件名/重复定义/数值范围
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("场景 name 不能为空".into());
        }
        if self.rooms.is_empty() {
            return Err("场景 rooms 不能为空".into());
        }
        if self.ticks == Some(0) {
            return Err("ticks 必须 > 0".into());
        }
        if self.cpu_limit() <= 0.0 {
            return Err(format!("cpu.limit 必须 > 0 (当前 {})", self.cpu_limit()));
        }
        if self.cpu_bucket() < 0.0 {
            return Err(format!("cpu.bucket 不能为负 (当前 {})", self.cpu_bucket()));
        }
        if let Some(n) = self.global_reset_every {
            if n == 0 {
                return Err("globalResetEvery 必须 > 0 (不设置表示不重置)".into());
            }
        }
        let mut rooms = std::collections::BTreeSet::new();
        for room in &self.rooms {
            if !is_valid_room_name(&room.name) {
                return Err(format!("非法房名 {:?} (应形如 W5N8 / E12S3)", room.name));
            }
            if !rooms.insert(room.name.clone()) {
                return Err(format!("房间 {} 重复定义", room.name));
            }
            if let Some(ctrl) = &room.controller {
                if ctrl.level > 8 {
                    return Err(format!("{} 控制器等级 {} 超出 0..8", room.name, ctrl.level));
                }
                if let Some(pos) = ctrl.pos {
                    check_pos(&room.name, "controller", pos)?;
                }
            }
            if let Some(SourcesSpec::Explicit { positions }) = &room.sources {
                if positions.is_empty() {
                    return Err(format!("{} sources.positions 不能为空", room.name));
                }
                for pos in positions {
                    check_pos(&room.name, "source", *pos)?;
                }
            }
            let mut creeps = std::collections::BTreeSet::new();
            for cr in &room.creeps {
                check_pos(&room.name, "creep", cr.pos)?;
                if cr.body.is_empty() {
                    return Err(format!("{} 单位 {} 身体不能为空", room.name, cr.name));
                }
                if cr.body.len() > constants::MAX_CREEP_SIZE {
                    return Err(format!(
                        "{} 单位 {} 部件数 {} 超过 {}",
                        room.name,
                        cr.name,
                        cr.body.len(),
                        constants::MAX_CREEP_SIZE
                    ));
                }
                for part in &cr.body {
                    if !constants::is_known_bodypart(part) {
                        return Err(format!("{} 单位 {} 含未知部件 {:?}", room.name, cr.name, part));
                    }
                }
                if !creeps.insert(cr.name.clone()) {
                    return Err(format!("{} 单位名 {} 重复", room.name, cr.name));
                }
            }
            for st in &room.structures {
                check_pos(&room.name, "structure", st.pos)?;
                if !constants::is_known_structure_type(&st.structure_type) {
                    return Err(format!("{} 未知结构类型 {:?}", room.name, st.structure_type));
                }
            }
            if let Some(TerrainSpec::Raw { data }) = &room.terrain {
                if data.len() != 2500 {
                    return Err(format!(
                        "{} terrain.raw 长度应为 2500, 实际 {}",
                        room.name,
                        data.len()
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn cpu_limit(&self) -> f64 {
        self.cpu.as_ref().and_then(|c| c.limit).unwrap_or(100.0)
    }

    pub fn cpu_bucket(&self) -> f64 {
        self.cpu.as_ref().and_then(|c| c.bucket).unwrap_or(10_000.0)
    }

    pub fn cpu_tick_limit(&self) -> f64 {
        self.cpu.as_ref().and_then(|c| c.tick_limit).unwrap_or_else(|| {
            let limit = self.cpu_limit();
            let bucket = self.cpu_bucket();
            (limit + bucket).min(500.0).max(limit)
        })
    }

    pub fn cpu_enforce(&self) -> bool {
        self.cpu.as_ref().map(|c| c.enforce).unwrap_or(false)
    }

    /// 全局重置间隔 (None / 0 表示不重置)
    pub fn global_reset_every(&self) -> Option<u64> {
        self.global_reset_every.filter(|n| *n > 0)
    }
}

/// 坐标必须落在 50×50 房间内
fn check_pos(room: &str, what: &str, pos: [u8; 2]) -> Result<(), String> {
    if pos[0] > 49 || pos[1] > 49 {
        return Err(format!("{} {} 坐标 [{}, {}] 超出 0..49", room, what, pos[0], pos[1]));
    }
    Ok(())
}

/// 房名形如 W5N8 / E12S3 (与引擎 apply_crossings 的解析保持一致)
fn is_valid_room_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(horiz) = chars.next() else { return false };
    if horiz != 'W' && horiz != 'E' {
        return false;
    }
    let rest: String = chars.collect();
    let Some((x, y)) = rest.split_once(['N', 'S']) else { return false };
    if x.is_empty() || y.is_empty() || x.len() > 3 || y.len() > 3 {
        return false;
    }
    x.chars().all(|c| c.is_ascii_digit()) && y.chars().all(|c| c.is_ascii_digit())
}

/// 场景 → 世界: 放置地形/Source/控制器/结构/单位
pub fn build_world(scenario: &Scenario) -> Result<World, String> {
    let seed = scenario.seed.unwrap_or(1);
    let mut world = World::new();
    for spec in &scenario.rooms {
        let terrain = build_terrain(spec, seed)?;
        let mut room = crate::engine::world::Room::new(&spec.name, terrain);
        // 保证关键位置可通行
        if let Some(ctrl) = &spec.controller {
            if let Some(pos) = ctrl.pos {
                room.terrain.set(pos[0] as usize, pos[1] as usize, 0);
                clear_walls_around(&mut room, pos[0], pos[1]);
            }
        }
        for st in &spec.structures {
            room.terrain.set(st.pos[0] as usize, st.pos[1] as usize, 0);
        }
        for cr in &spec.creeps {
            room.terrain.set(cr.pos[0] as usize, cr.pos[1] as usize, 0);
        }

        // 控制器
        if let Some(ctrl) = &spec.controller {
            let pos = ctrl.pos.unwrap_or([25, 25]);
            let id = world.alloc_id();
            let mut obj = objects::controller(id, pos[0], pos[1], ctrl.level);
            obj.mine = true;
            obj.progress = ctrl.progress.unwrap_or(0);
            room.objects.insert(id, obj);
        }

        // Source
        let source_capacity = constants::SOURCE_ENERGY_CAPACITY;
        match &spec.sources {
            None => {
                let count = 2usize;
                let energy = source_capacity;
                let positions = auto_source_positions(&room, count);
                for pos in positions {
                    let id = alloc_id(&mut world.next_id);
                    room.objects.insert(id, objects::source(id, pos[0], pos[1], energy));
                    clear_walls_around(&mut room, pos[0], pos[1]);
                }
            }
            Some(SourcesSpec::Auto { count, energy }) => {
                let count = count.unwrap_or(2).min(3) as usize;
                let energy = energy.unwrap_or(source_capacity);
                let positions = auto_source_positions(&room, count);
                for pos in positions {
                    let id = world.alloc_id();
                    room.objects.insert(id, objects::source(id, pos[0], pos[1], energy));
                    // 保证 source 格可达: 周围开洞
                    clear_walls_around(&mut room, pos[0], pos[1]);
                }
            }
            Some(SourcesSpec::Explicit { positions }) => {
                let energy = source_capacity;
                for pos in positions {
                    let id = alloc_id(&mut world.next_id);
                    room.objects.insert(id, objects::source(id, pos[0], pos[1], energy));
                    clear_walls_around(&mut room, pos[0], pos[1]);
                }
            }
        }

        // 结构
        let mut spawn_index = 0u32;
        for st in &spec.structures {
            let id = alloc_id(&mut world.next_id);
            let mut obj = objects::structure(
                id,
                &st.structure_type,
                st.pos[0],
                st.pos[1],
                constants::is_owned_structure(&st.structure_type),
            );
            if let Some(e) = st.energy {
                obj.store.insert("energy".into(), e);
            }
            if let Some(h) = st.hits {
                obj.hits = h;
            }
            match st.structure_type.as_str() {
                "spawn" => {
                    obj.energy_capacity = constants::SPAWN_ENERGY_CAPACITY;
                    spawn_index += 1;
                    obj.name = Some(format!("Spawn{spawn_index}"));
                }
                "storage" => obj.energy_capacity = constants::STORAGE_CAPACITY,
                "container" => obj.energy_capacity = constants::CONTAINER_CAPACITY,
                "tower" => obj.energy_capacity = constants::TOWER_CAPACITY,
                "terminal" => obj.energy_capacity = constants::TERMINAL_CAPACITY,
                "link" => obj.energy_capacity = constants::LINK_CAPACITY,
                _ => {}
            }
            room.objects.insert(id, obj);
        }

        // 单位
        for cr in &spec.creeps {
            let id = world.alloc_id();
            let body: Vec<BodyPart> = cr
                .body
                .iter()
                .map(|p| BodyPart { part_type: p.clone(), hits: 100 })
                .collect();
            let mut obj = objects::creep_obj(id, &cr.name, cr.pos[0], cr.pos[1], body);
            obj.store = cr.store.clone();
            if let Some(ttl) = cr.ttl {
                obj.age_time = Some(world.tick + ttl);
            }
            room.objects.insert(id, obj);
        }

        world.rooms.insert(spec.name.clone(), room);
    }
    prime_world(&mut world);
    Ok(world)
}

fn build_terrain(spec: &RoomSpec, seed: u64) -> Result<Terrain, String> {
    match &spec.terrain {
        Some(TerrainSpec::Flat) | None => Ok(Terrain::plain()),
        Some(TerrainSpec::Generated { seed: s }) => Ok(terrain::generate(s.unwrap_or(seed))),
        Some(TerrainSpec::Rows { rows }) => {
            if rows.len() != 50 {
                return Err(format!("{} terrain.rows 需要 50 行", spec.name));
            }
            let mut data = String::with_capacity(2500);
            for (i, row) in rows.iter().enumerate() {
                if row.len() != 50 {
                    return Err(format!("{} terrain.rows[{i}] 需要 50 字符", spec.name));
                }
                for ch in row.chars() {
                    data.push(match ch {
                        '.' | '0' => '0',
                        '#' | '1' => '1',
                        '~' | '2' => '2',
                        _ => return Err(format!("{} terrain.rows 含非法字符 {ch}", spec.name)),
                    });
                }
            }
            Terrain::parse(&data).ok_or_else(|| "地形解析失败".to_string())
        }
        Some(TerrainSpec::Raw { data }) => {
            Terrain::parse(data).ok_or_else(|| format!("{} terrain.raw 需要 2500 字符", spec.name))
        }
    }
}

/// 清理 (x,y) 周围 3×3 的墙 (保证关键点位可达)
fn clear_walls_around(room: &mut crate::engine::world::Room, x: u8, y: u8) {
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if (0..50).contains(&nx) && (0..50).contains(&ny) {
                if room.terrain.is_wall(nx as usize, ny as usize) {
                    room.terrain.set(nx as usize, ny as usize, 0);
                }
            }
        }
    }
}

/// 自动放 source: 中线两侧对称搜索非墙格
fn auto_source_positions(room: &crate::engine::world::Room, count: usize) -> Vec<[u8; 2]> {
    let mut positions = Vec::new();
    let candidates = [
        ([10, 10], [10, 40]),
        ([40, 10], [40, 40]),
        ([25, 12], [25, 38]),
    ];
    'outer: for pair in candidates.iter().take(count) {
        for base in [pair.0, pair.1] {
            let mut found = None;
            for dy in 0..6i32 {
                for dx in 0..6i32 {
                    let (x, y) = (base[0] as i32 + dx - 3, base[1] as i32 + dy - 3);
                    if (0..50).contains(&x) && (0..50).contains(&y) && !room.terrain.is_wall(x as usize, y as usize)
                        && !positions.iter().any(|p: &[u8; 2]| p[0] as i32 == x && p[1] as i32 == y)
                    {
                        found = Some([x as u8, y as u8]);
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            if let Some(pos) = found {
                positions.push(pos);
                if positions.len() >= count {
                    break 'outer;
                }
            }
        }
    }
    positions
}
