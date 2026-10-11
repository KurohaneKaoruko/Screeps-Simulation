//! 场景定义与加载: JSON → World 初始状态。

use crate::constants;
use crate::objects::{self, BodyPart};
use crate::terrain::{self, Terrain};
use crate::world::{prime_world, World};
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

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum SourcesSpec {
    Auto { count: Option<u32>, energy: Option<u32> },
    Explicit { positions: Vec<[u8; 2]> },
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
    #[serde(default)]
    pub rooms: Vec<RoomSpec>,
    #[serde(default)]
    pub asserts: Vec<AssertSpec>,
}

fn default_true() -> bool {
    true
}

impl Scenario {
    pub fn from_json(text: &str) -> Result<Scenario, String> {
        serde_json::from_str(text).map_err(|e| format!("场景解析失败: {e}"))
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
}

/// 场景 → 世界: 放置地形/Source/控制器/结构/单位
pub fn build_world(scenario: &Scenario) -> Result<World, String> {
    let seed = scenario.seed.unwrap_or(1);
    let mut world = World::new();
    for spec in &scenario.rooms {
        let terrain = build_terrain(spec, seed)?;
        let mut room = crate::world::Room::new(&spec.name, terrain);
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
            None | Some(SourcesSpec::Auto { count, energy }) => {
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
                    let id = world.alloc_id();
                    room.objects.insert(id, objects::source(id, pos[0], pos[1], energy));
                    clear_walls_around(&mut room, pos[0], pos[1]);
                }
            }
        }

        // 结构
        for st in &spec.structures {
            let id = world.alloc_id();
            let mut obj = objects::structure(id, &st.structure_type, st.pos[0], st.pos[1], true);
            if let Some(e) = st.energy {
                obj.store.insert("energy".into(), e);
            }
            if let Some(h) = st.hits {
                obj.hits = h;
            }
            match st.structure_type.as_str() {
                "spawn" => {
                    obj.energy_capacity = constants::SPAWN_ENERGY_CAPACITY;
                    obj.name = Some(format!("Spawn1"));
                }
                "storage" => obj.energy_capacity = constants::STORAGE_CAPACITY,
                "container" => obj.energy_capacity = constants::CONTAINER_CAPACITY,
                "tower" => obj.energy_capacity = constants::TOWER_CAPACITY,
                "terminal" => obj.energy_capacity = constants::TERMINAL_CAPACITY,
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
fn clear_walls_around(room: &mut crate::world::Room, x: u8, y: u8) {
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
fn auto_source_positions(room: &crate::world::Room, count: usize) -> Vec<[u8; 2]> {
    let mut positions = Vec::new();
    let candidates = [
        ([10, 10], [10, 40]),
        ([40, 10], [40, 40]),
        ([25, 12], [25, 38]),
    ];
    'outer: for pair in candidates.iter().take(count) {
        for base in pair {
            let mut found = None;
            for dy in 0..6i32 {
                for dx in 0..6i32 {
                    let (x, y) = (base[0] as i32 + dx - 3, base[1] as i32 + dy - 3);
                    if (0..50).contains(&x) && (0..50).contains(&y) && !room.terrain.is_wall(x as usize, y as usize)
                        && !positions.iter().any(|p| p[0] as i32 == x && p[1] as i32 == y)
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
