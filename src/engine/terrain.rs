//! 房间地形: 50×50, 每格 0=平原 1=墙 2=沼泽。
//! 支持场景显式给定 (每行字符串) 或种子生成 (简单噪声, 不追求复刻官方地图生成算法)。

use crate::engine::constants::{TERRAIN_MASK_SWAMP, TERRAIN_MASK_WALL};

pub const ROOM_SIZE: usize = 50;
pub const TILES: usize = ROOM_SIZE * ROOM_SIZE;

#[derive(Clone)]
pub struct Terrain {
    pub data: [u8; TILES],
}

impl Terrain {
    pub fn plain() -> Terrain {
        Terrain { data: [0; TILES] }
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.data[y * ROOM_SIZE + x]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, value: u8) {
        self.data[y * ROOM_SIZE + x] = value;
    }

    pub fn is_wall(&self, x: usize, y: usize) -> bool {
        self.get(x, y) & TERRAIN_MASK_WALL != 0
    }

    pub fn is_swamp(&self, x: usize, y: usize) -> bool {
        self.get(x, y) & TERRAIN_MASK_SWAMP != 0
    }

    /// 序列化为 2500 字符的字符串 (快照传给 JS shim)
    pub fn to_string(&self) -> String {
        self.data.iter().map(|b| (b'0' + b) as char).collect()
    }

    /// 从 2500 字符解析
    pub fn parse(s: &str) -> Option<Terrain> {
        let bytes = s.as_bytes();
        if bytes.len() != TILES {
            return None;
        }
        let mut data = [0u8; TILES];
        for (i, b) in bytes.iter().enumerate() {
            data[i] = match b {
                b'0' => 0,
                b'1' => TERRAIN_MASK_WALL,
                b'2' => TERRAIN_MASK_SWAMP,
                b'3' => TERRAIN_MASK_WALL | TERRAIN_MASK_SWAMP,
                _ => return None,
            };
        }
        Some(Terrain { data })
    }

    /// 50 行可视化文本
    pub fn render(&self) -> String {
        (0..ROOM_SIZE)
            .map(|y| {
                (0..ROOM_SIZE)
                    .map(|x| match self.get(x, y) {
                        0 => '.',
                        1 => '#',
                        2 => '~',
                        _ => '?',
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 确定性伪随机 (xorshift64*), 场景种子驱动
#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 { 0x9E3779B97F4A7C15 } else { seed })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// 种子生成地形: 值噪声 + 分位数阈值 → 墙带 ~12% / 沼泽 ~28%。
/// 生成规则偏"竞技地图", 不模拟官方地形算法的岩壁走向。
pub fn generate(seed: u64) -> Terrain {
    let mut rng = Rng::new(seed);
    let mut elevation = [0f64; TILES];
    let mut moisture = [0f64; TILES];
    let nodes: Vec<(usize, usize, f64)> = (0..10)
        .map(|_| {
            (
                rng.below(ROOM_SIZE as u64) as usize,
                rng.below(ROOM_SIZE as u64) as usize,
                rng.next_f64(),
            )
        })
        .collect();
    let moisture_nodes: Vec<(usize, usize, f64)> = (0..8)
        .map(|_| {
            (
                rng.below(ROOM_SIZE as u64) as usize,
                rng.below(ROOM_SIZE as u64) as usize,
                rng.next_f64(),
            )
        })
        .collect();
    let field = |nodes: &[(usize, usize, f64)], x: usize, y: usize| -> f64 {
        let mut v = 0.0;
        let mut wsum = 0.0;
        for &(nx, ny, val) in nodes {
            let dx = (nx as f64 - x as f64).abs();
            let dy = (ny as f64 - y as f64).abs();
            let d = (dx * dx + dy * dy).sqrt().max(1.0);
            let w = 1.0 / (d * d);
            v += val * w;
            wsum += w;
        }
        v / wsum
    };
    for y in 0..ROOM_SIZE {
        for x in 0..ROOM_SIZE {
            elevation[y * ROOM_SIZE + x] = field(&nodes, x, y);
            moisture[y * ROOM_SIZE + x] = field(&moisture_nodes, x, y);
        }
    }
    // 分位数阈值: 墙取海拔 [55%, 67%) 分位带 (≈12%), 沼泽取湿度前 40% (≈28%)
    let mut e_sorted = elevation;
    e_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut m_sorted = moisture;
    m_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let quantile = |arr: &[f64], q: f64| arr[((TILES as f64 - 1.0) * q) as usize];
    let wall_low = quantile(&e_sorted, 0.55);
    let wall_high = quantile(&e_sorted, 0.67);
    let swamp_threshold = quantile(&m_sorted, 0.60);

    let mut terrain = Terrain::plain();
    for y in 0..ROOM_SIZE {
        for x in 0..ROOM_SIZE {
            let e = elevation[y * ROOM_SIZE + x];
            let m = moisture[y * ROOM_SIZE + x];
            let tile = if e >= wall_low && e < wall_high {
                TERRAIN_MASK_WALL
            } else if m >= swamp_threshold {
                TERRAIN_MASK_SWAMP
            } else {
                0
            };
            terrain.set(x, y, tile);
        }
    }
    terrain
}
