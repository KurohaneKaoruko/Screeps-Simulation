//! 移动结算: 移植官方 engine src/processor/intents/movement.js 的语义。
//! 1) move 意图登记目标格 → 2) check() 碰撞仲裁 → 3) creep tick 时 execute() 应用位移与疲劳。
//! 疲劳规则: (非 MOVE/CARRY 部件 + 按负载折算的 CARRY 件数) × 地形成本 (路1/平原2/沼泽10),
//! 每 tick 恢复 2 × 活跃 MOVE。

use crate::engine::constants;
use crate::engine::objects::Obj;
use crate::engine::world::Room;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Movement {
    /// (房名, 目标格) → 想要进入的对象 (登记顺序)。按房名隔离,
    /// 否则多房间同坐标会互相冲突, 且后处理房间会取消前面房间的移动。
    matrix: BTreeMap<(String, u8, u8), Vec<u64>>,
    /// 仲裁后每个对象的落点 (对象 id 全局唯一, 无需按房隔离)
    resolved: BTreeMap<u64, (u8, u8)>,
    /// (房名, 格) 竞争计数 (受压优先仲裁用)
    affected_cnt: BTreeMap<(String, u8, u8), usize>,
    /// move 意图提交时的疲劳快照 (engine: _oldFatigue, 提交时疲劳非 0 则不可动)
    old_fatigue: BTreeMap<u64, u32>,
    /// 拉拽关系 (官方 object._pull / target._pulled): 拉人者 → 被拉者, 被拉者 → 拉人者
    pull: BTreeMap<u64, u64>,
    pulled: BTreeMap<u64, u64>,
    /// 本 tick 的疲劳累计 (官方 object._fatigue, tick 内可为负, 落库时取 max(0, ·))
    fatigue_acc: BTreeMap<u64, i64>,
}

impl Movement {
    pub fn init(&mut self) {
        self.matrix.clear();
        self.resolved.clear();
        self.affected_cnt.clear();
        self.old_fatigue.clear();
        self.pull.clear();
        self.pulled.clear();
        self.fatigue_acc.clear();
    }

    /// 官方 movement.addPulling: 拉拽链不能成环 (A 拉 B、B 拉 A 时后者被忽略)
    pub fn add_pulling(&mut self, room: &Room, puller: u64, target: u64) {
        let mut t = target;
        for _ in 0..1000 {
            if t == puller {
                return;
            }
            match self.pull.get(&t) {
                Some(&next) if room.objects.contains_key(&next) => t = next,
                _ => break,
            }
        }
        self.pull.insert(puller, target);
        self.pulled.insert(target, puller);
    }

    /// 官方 creeps/_add-fatigue.js: 负值 (MOVE 恢复) 先抵自己的疲劳, 剩余部分与正值 (移动消耗)
    /// 都沿 _pulled 链交给最前面的拉人者 —— 被拉者的移动疲劳由拉它的 creep 承担。
    pub fn add_fatigue(&mut self, room: &mut Room, id: u64, mut d: i64) {
        let Some(own) = room.objects.get(&id).map(|o| o.fatigue as i64) else { return };
        let own = *self.fatigue_acc.entry(id).or_insert(own);
        if own > 0 && d < 0 {
            let resting = own.min(-d);
            let left = own - resting;
            self.fatigue_acc.insert(id, left);
            if let Some(o) = room.objects.get_mut(&id) {
                o.fatigue = left.max(0) as u32;
            }
            d += resting;
            if d == 0 {
                return;
            }
        }
        let mut head = id;
        for _ in 0..1000 {
            match self.pulled.get(&head) {
                Some(&p) if room.objects.contains_key(&p) => head = p,
                _ => break,
            }
        }
        let Some(base) = room.objects.get(&head).map(|o| o.fatigue as i64) else { return };
        let acc = self.fatigue_acc.entry(head).or_insert(base);
        *acc += d;
        let f = (*acc).max(0) as u32;
        if let Some(o) = room.objects.get_mut(&head) {
            o.fatigue = f;
        }
    }

    pub fn add(&mut self, room_name: &str, id: u64, x: u8, y: u8, old_fatigue: u32) {
        let key = (room_name.to_string(), x, y);
        self.matrix.entry(key.clone()).or_default().push(id);
        *self.affected_cnt.entry(key).or_insert(0) += 1;
        self.old_fatigue.insert(id, old_fatigue);
    }

    /// 目标格是否有对象正在进入 (出生放置用)
    pub fn is_tile_busy(&self, room_name: &str, x: u8, y: u8) -> bool {
        self.matrix.contains_key(&(room_name.to_string(), x, y))
    }

    /// 官方 canMove: 被拉的 creep 不看疲劳和 MOVE 部件
    fn can_move(&self, id: u64, obj: &Obj) -> bool {
        self.pulled.contains_key(&id)
            || self.old_fatigue.get(&id).copied().unwrap_or(0) == 0 && obj.live_parts("move") > 0
    }

    /// 目标格是否存在阻挡: 静止 creep / 阻挡结构 / 墙 (有路除外)
    fn check_obstacle_at(
        room: &Room,
        x: usize,
        y: usize,
        resolved: &BTreeMap<u64, (u8, u8)>,
    ) -> bool {
        let mut has_road = false;
        for o in room.objects.values() {
            if o.x as usize != x || o.y as usize != y {
                continue;
            }
            match o.obj_type.as_str() {
                "creep" => {
                    // 站在目标格且本 tick 未获准离开的 creep 是阻挡
                    if !resolved.contains_key(&o.id) {
                        return true;
                    }
                }
                "road" => has_road = true,
                t if constants::OBSTACLE_OBJECT_TYPES.contains(&t) => return true,
                "constructionSite" => {
                    if o.is_obstacle() {
                        return true;
                    }
                }
                _ => {}
            }
        }
        room.terrain.is_wall(x, y) && !has_road
    }

    /// 碰撞仲裁: 官方 rate1(交换对象)→rate2(被拉)→rate3(拉人)→rate4(移动比) 排序取每格胜者,
    /// 败者与被阻挡者不移动, 且连锁取消要进入其原格的对象。
    /// 只处理 `room_name` 房间的登记; 每个房间各自 `check` (官方 processor.js:225,324)。
    pub fn check(&mut self, room_name: &str, room: &Room) {
        let rn = room_name.to_string();
        // 第一轮: 每个目标格选出胜者
        for ((mrn, kx, ky), candidates) in &self.matrix {
            if mrn != &rn {
                continue;
            }
            let key = (*kx, *ky);
            let winner = if candidates.len() > 1 {
                let mut rated: Vec<(u32, u32, u32, f64, u64)> = candidates
                    .iter()
                    .filter_map(|id| room.objects.get(id).map(|obj| (*id, obj)))
                    .map(|(id, obj)| {
                        let origin = (obj.x, obj.y);
                        // rate1: 交换对象 —— matrix[我的起点] 里有正站在争夺格 (key) 上的 creep,
                        // 即对方要进我的起点、我要进对方的格子 (官方 movement.js:124-128)
                        let rate1 = if self
                            .matrix
                            .get(&(rn.clone(), origin.0, origin.1))
                            .is_some_and(|arr| {
                                arr.iter().any(|&other| {
                                    room.objects
                                        .get(&other)
                                        .is_some_and(|o| o.x == key.0 && o.y == key.1)
                                })
                            })
                        {
                            100
                        } else {
                            self.affected_cnt
                                .get(&(rn.clone(), origin.0, origin.1))
                                .copied()
                                .unwrap_or(0) as u32
                        };
                        // 官方 movement.js:118: calcBodyEffectiveness(MOVE, 'fatigue', 1) / 负重
                        let rate4 = obj.calc_body_effectiveness("move", "fatigue", 1) as f64
                            / move_weight(obj).max(1) as f64;
                        let rate2 = self.pulled.contains_key(&id) as u32;
                        let rate3 = self.pull.contains_key(&id) as u32;
                        (rate1, rate2, rate3, rate4, id)
                    })
                    .collect();
                rated.sort_by(|a, b| {
                    b.0.cmp(&a.0)
                        .then(b.1.cmp(&a.1))
                        .then(b.2.cmp(&a.2))
                        .then(b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal))
                });
                rated.first().map(|r| r.4)
            } else {
                candidates.first().copied()
            };
            if let Some(w) = winner {
                self.resolved.insert(w, key);
            }
        }

        // 第二轮: 不可动 / 目标被占 → 移除并连锁 (只处理本房间的已决移动)
        let ids: Vec<u64> = self
            .resolved
            .keys()
            .filter(|id| room.objects.contains_key(id))
            .copied()
            .collect();
        for id in ids {
            let Some(&key) = self.resolved.get(&id) else { continue };
            // 官方: 被拉者必须正走进拉人者当前所在格, 且拉人者确实拉的是它, 否则拉拽关系作废
            if let Some(&p) = self.pulled.get(&id) {
                let valid = self.pull.get(&p) == Some(&id)
                    && room.objects.get(&p).is_some_and(|po| (po.x, po.y) == key);
                if !valid {
                    self.pulled.remove(&id);
                    if self.pull.get(&p) == Some(&id) {
                        self.pull.remove(&p);
                    }
                }
            }
            let blocked = room
                .objects
                .get(&id)
                .map(|obj| {
                    !self.can_move(id, obj)
                        || Self::check_obstacle_at(room, key.0 as usize, key.1 as usize, &self.resolved)
                })
                .unwrap_or(true);
            if blocked {
                Self::remove_from_matrix(room, &mut self.resolved, key);
            }
        }
    }

    /// 移除一个已裁决的移动并连锁取消进入其原格的移动
    fn remove_from_matrix(
        room: &Room,
        resolved: &mut BTreeMap<u64, (u8, u8)>,
        key: (u8, u8),
    ) {
        // resolved 跨房间共用 (对象 id 全局唯一), 按坐标查找时必须限定在本房间,
        // 否则会取消先仲裁的另一个房间里同坐标的移动, 而本房被挡的 creep 照样走进占用格。
        let in_room = |id: &u64| room.objects.contains_key(id);
        let object_id = match resolved.iter().find(|(id, &k)| k == key && in_room(id)) {
            Some((&id, _)) => id,
            None => return,
        };
        resolved.remove(&object_id);
        if let Some(obj) = room.objects.get(&object_id) {
            let origin = (obj.x, obj.y);
            if origin != key && resolved.iter().any(|(id, &k)| k == origin && in_room(id)) {
                Self::remove_from_matrix(room, resolved, origin);
            }
        }
    }

    /// creep tick: 应用位移/疲劳/道路磨损; 返回是否实际移动
    pub fn execute(&mut self, room: &mut Room, id: u64) -> bool {
        let Some(&(nx, ny)) = self.resolved.get(&id) else {
            return false;
        };
        let (at_edge_before, parts_len) = {
            let Some(obj) = room.objects.get(&id) else {
                return false;
            };
            if !self.can_move(id, obj) {
                return false;
            }
            (
                obj.x == 0 || obj.x == 49 || obj.y == 0 || obj.y == 49,
                obj.body.len() as u32,
            )
        };
        let (nxi, nyi) = (nx as usize, ny as usize);
        let has_road = room
            .objects
            .values()
            .any(|o| o.obj_type == "road" && o.x as usize == nxi && o.y as usize == nyi);

        // 目标格地形成本: 路1 / 沼泽10 / 平原2 (engine: fatigueRate)
        let fatigue_rate = if has_road {
            1
        } else if room.terrain.is_swamp(nxi, nyi) {
            10
        } else {
            2
        };

        let obj = room.objects.get_mut(&id).unwrap();
        let at_edge_after = nx == 0 || nx == 49 || ny == 0 || ny == 49;
        obj.x = nx;
        obj.y = ny;
        if at_edge_after && !at_edge_before {
            // 从房内进入边缘格: 不加疲劳 (跨房过渡)
            obj.fatigue = 0;
            self.fatigue_acc.insert(id, 0);
        } else {
            let d = (fatigue_rate * move_weight(obj)) as i64;
            self.add_fatigue(room, id, d);
        }

        // 道路磨损: 提前到期 nextDecayTime (等价官方 nextDecayTime -= wear)
        if has_road {
            for o in room.objects.values_mut() {
                if o.obj_type == "road" && o.x as usize == nxi && o.y as usize == nyi {
                    if let Some(t) = o.next_decay_time {
                        o.next_decay_time = Some(t.saturating_sub(parts_len as u64));
                    }
                    break;
                }
            }
        }
        true
    }
}

/// 移动重量: 非 MOVE/CARRY 部件全重 + 按负载折算占用的 CARRY 件数 (calcResourcesWeight)
fn move_weight(obj: &Obj) -> u32 {
    let mut remaining = obj.store.values().sum::<u32>();
    let mut weight = obj
        .body
        .iter()
        .filter(|p| p.part_type != "move" && p.part_type != "carry")
        .count() as u32;
    for part in obj.body.iter().rev() {
        if remaining == 0 {
            break;
        }
        if part.part_type != "carry" || part.hits == 0 {
            continue;
        }
        // 官方 calcResourcesWeight: 强化 CARRY 一件可承载 50 × capacity 倍率
        let boost = part
            .boost
            .as_deref()
            .and_then(|b| constants::boost_effect("carry", b, "capacity"))
            .unwrap_or(1.0);
        let take = remaining.min((constants::CARRY_CAPACITY as f64 * boost) as u32);
        remaining -= take;
        weight += 1;
    }
    weight
}

/// 每 tick 疲劳恢复: 2 × 活跃 MOVE (含 boost 倍率)
pub fn recover_fatigue(obj: &mut Obj) {
    let moves = obj.calc_body_effectiveness("move", "fatigue", 1);
    if moves > 0 {
        obj.fatigue = obj.fatigue.saturating_sub(2 * moves);
    }
}

/// 出生放置: spawn 周围 8 向第一格无阻挡位 (engine: _born-creep)。
/// `directions` 非空时按官方 `setDirections` 语义只在该优先序内挑格。
pub fn find_birth_tile(
    room: &Room,
    spawn: &Obj,
    movement: &Movement,
    directions: Option<&[i32]>,
) -> Option<(u8, u8)> {
    const DIRS: [(i32, i32); 8] = [
        (0, -1),
        (1, -1),
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
    ];
    let candidates: Vec<(i32, i32)> = match directions {
        Some(dirs) if !dirs.is_empty() => dirs
            .iter()
            .filter_map(|d| DIRS.get((*d - 1).max(0) as usize).copied())
            .collect(),
        _ => DIRS.to_vec(),
    };
    for (dx, dy) in candidates {
        let nx = spawn.x as i32 + dx;
        let ny = spawn.y as i32 + dy;
        if !(0..50).contains(&nx) || !(0..50).contains(&ny) {
            continue;
        }
        let (nx, ny) = (nx as u8, ny as u8);
        if room.terrain.is_wall(nx as usize, ny as usize) {
            continue;
        }
        if movement.is_tile_busy(&room.name, nx, ny) {
            continue;
        }
        let blocked = room.objects.values().any(|o| {
            o.x == nx && o.y == ny && (o.is_obstacle() || o.obj_type == "creep")
        });
        if !blocked {
            return Some((nx, ny));
        }
    }
    None
}
