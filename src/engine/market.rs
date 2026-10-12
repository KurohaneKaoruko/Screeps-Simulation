//! 市场 (Game.market): 订单 / 成交 / credits / 终端跨房交易。
//!
//! 官方语义 (见 docs.screeps.com/market.html):
//! - 创建任何订单都要缴纳「订单总价 × 5%」的 credits 手续费 (下单时立即扣);
//! - 成交时资源始终从**卖方**终端流向**买方**终端, credits 从**买方**流向**卖方**;
//! - 运输能量费无论买单还是卖单, 一律从**接单方**终端扣除 (calcTransactionCost);
//! - 卖单资源不足 / 买单 credits 不足时订单转为未激活 (active=false)。
//!
//! 单玩家模拟器里的对手方是"外部订单"(NPC 或他玩家, `external=true`): 其终端与
//! 账户不在世界内, 成交时只结算**接单方**(本玩家)一侧的资源/credits/能量。

use std::collections::BTreeMap;

use crate::engine::constants;
use crate::engine::intents::{self, store_add};
use crate::engine::objects;
use crate::engine::world::{Transaction, World, TRANSACTIONS_KEEP};

/// 官方 market 手续费比例 (订单总价的 5%)
pub const MARKET_FEE: f64 = 0.05;
/// 订单单价下限 (官方 0.001 Cr)
pub const MARKET_MIN_PRICE: f64 = 0.001;
/// 订单单价上限 (官方 1e8 Cr)
pub const MARKET_MAX_PRICE: f64 = 100_000_000.0;
/// 订单 id 基址 (与对象 id 区分, 保证确定性且不撞车)
const ORDER_ID_BASE: u64 = 0x9e00_0000_0000_0000;
/// 保留的成交记录上限 (新→旧)
pub const MARKET_DEALS_KEEP: usize = 2000;

/// 市场订单 (官方 Game.market 订单的字段子集)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MarketOrder {
    pub id: String,
    /// "buy" | "sell"
    pub order_type: String,
    pub resource_type: String,
    pub room_name: String,
    pub price: f64,
    /// 剩余可成交数量
    pub amount: u32,
    /// 初始总量 (extendOrder 会累加)
    pub total_amount: u32,
    pub created: u64,
    /// true = 外部订单 (NPC/他玩家, 由场景给出); false = 本玩家创建的订单
    #[serde(default)]
    pub external: bool,
}

/// 一笔成交记录 (供 Game.market.getHistory 聚合)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MarketDeal {
    pub tick: u64,
    pub resource_type: String,
    pub amount: u32,
    pub price: f64,
}

/// 本 tick 登记的市场操作 (官方 market 方法在 tick 末统一结算)
#[derive(Clone, Debug)]
pub enum MarketOp {
    Create {
        order_type: String,
        resource_type: String,
        room_name: String,
        price: f64,
        total_amount: u32,
    },
    Deal {
        order_id: String,
        amount: u32,
        room_name: Option<String>,
    },
    Cancel {
        order_id: String,
    },
    ChangePrice {
        order_id: String,
        new_price: f64,
    },
    Extend {
        order_id: String,
        add_amount: u32,
    },
}

/// 确定性订单 id (24 位 hex 风格, 与官方 ObjectId 观感一致)
pub fn order_id(seq: u64) -> String {
    format!("{:024x}", ORDER_ID_BASE.wrapping_add(seq))
}

/// 单价规范化到 3 位小数 (官方订单价格最多 3 位小数)
pub fn normalize_price(price: f64) -> f64 {
    (price * 1000.0).round() / 1000.0
}

/// 单价是否合法 (有限且落在 [0.001, 1e8])
pub fn valid_price(price: f64) -> bool {
    price.is_finite() && price >= MARKET_MIN_PRICE && price <= MARKET_MAX_PRICE
}

/// 本玩家在某房间终端的某资源存量 (无则 0)
fn our_terminal_stock(world: &World, room_name: &str, resource: &str) -> u32 {
    world
        .rooms
        .get(room_name)
        .and_then(|r| r.objects.values().find(|o| o.obj_type == "terminal" && o.mine))
        .map(|t| t.store.get(resource).copied().unwrap_or(0))
        .unwrap_or(0)
}

/// 订单是否激活 (官方: 卖单资源不足 / 买单 credits 不足 → 未激活; 外部订单恒激活)
pub fn is_active(world: &World, order: &MarketOrder) -> bool {
    if order.external {
        return true;
    }
    match order.order_type.as_str() {
        "buy" => world.market_credits + 1e-9 >= order.price * order.amount as f64,
        "sell" => our_terminal_stock(world, &order.room_name, &order.resource_type) >= order.amount,
        _ => false,
    }
}

/// 结算本 tick 登记的市场操作 (按提交顺序, 官方 market 阶段)
pub fn execute_market_ops(world: &mut World, ops: &[MarketOp]) {
    for op in ops {
        match op {
            MarketOp::Create {
                order_type,
                resource_type,
                room_name,
                price,
                total_amount,
            } => create_order(world, order_type, resource_type, room_name, *price, *total_amount),
            MarketOp::Deal { order_id, amount, room_name } => {
                deal(world, order_id, *amount, room_name.as_deref())
            }
            MarketOp::Cancel { order_id } => {
                if world.market_orders.get(order_id).is_some_and(|o| !o.external) {
                    world.market_orders.remove(order_id);
                }
            }
            MarketOp::ChangePrice { order_id, new_price } => {
                let np = normalize_price(*new_price);
                if let Some(o) = world.market_orders.get_mut(order_id) {
                    if !o.external && valid_price(np) {
                        o.price = np;
                    }
                }
            }
            MarketOp::Extend { order_id, add_amount } => {
                if let Some(o) = world.market_orders.get_mut(order_id) {
                    if !o.external {
                        o.total_amount = o.total_amount.saturating_add(*add_amount);
                        o.amount = o.amount.saturating_add(*add_amount);
                    }
                }
            }
        }
    }
}

fn create_order(
    world: &mut World,
    order_type: &str,
    resource_type: &str,
    room_name: &str,
    price: f64,
    total_amount: u32,
) {
    if order_type != "buy" && order_type != "sell" {
        return;
    }
    if !constants::RESOURCES_ALL.contains(&resource_type) || !intents::is_room_name(room_name) {
        return;
    }
    if total_amount == 0 || !valid_price(price) {
        return;
    }
    let price = normalize_price(price);
    let fee = price * total_amount as f64 * MARKET_FEE;
    if world.market_credits + 1e-9 < fee {
        return;
    }
    world.market_credits -= fee;
    let seq = world.market_order_seq;
    world.market_order_seq = world.market_order_seq.wrapping_add(1);
    let id = order_id(seq);
    let created = world.tick;
    world.market_orders.insert(
        id.clone(),
        MarketOrder {
            id,
            order_type: order_type.to_string(),
            resource_type: resource_type.to_string(),
            room_name: room_name.to_string(),
            price,
            amount: total_amount,
            total_amount,
            created,
            external: false,
        },
    );
}

/// 执行一笔成交 (接单方 = 本玩家; 对手方外部则只结算我方一侧)
fn deal(world: &mut World, order_id: &str, amount: u32, your_room: Option<&str>) {
    let Some(order) = world.market_orders.get(order_id).cloned() else { return };
    if amount == 0 || amount > order.amount {
        return;
    }
    let tick = world.tick;
    // 接单方终端所在房间: 指定房间优先, 否则取本玩家唯一有终端的房间
    let Some(taker_room) = resolve_taker_room(world, your_room) else { return };
    let Some(taker_id) = our_terminal_id(world, &taker_room) else { return };
    {
        let t = &world.rooms[&taker_room].objects[&taker_id];
        if t.cooldown_time > tick {
            return;
        }
    }
    let range = intents::calc_rooms_distance(&taker_room, &order.room_name, world.world_size());
    let cost = intents::terminal_energy_cost(amount, range);
    let price_total = order.price * amount as f64;
    let res = order.resource_type.as_str();

    if order.order_type == "sell" {
        // 订单在卖, 我们买入: 资源进入我们的终端; 我们付 credits + 能量费
        let (free, energy) = {
            let t = &world.rooms[&taker_room].objects[&taker_id];
            (
                t.store_capacity().saturating_sub(objects::store_sum(&t.store)),
                t.store.get("energy").copied().unwrap_or(0),
            )
        };
        if free < amount || energy < cost || world.market_credits + 1e-9 < price_total {
            return;
        }
        {
            let t = world
                .rooms
                .get_mut(&taker_room)
                .unwrap()
                .objects
                .get_mut(&taker_id)
                .unwrap();
            store_add(&mut t.store, res, amount as i64);
            store_add(&mut t.store, "energy", -(cost as i64));
            t.cooldown_time = tick + constants::TERMINAL_COOLDOWN;
        }
        world.market_credits -= price_total;
    } else {
        // 订单在买, 我们卖出: 资源离开我们的终端; 我们收 credits, 付能量费
        let (have, energy) = {
            let t = &world.rooms[&taker_room].objects[&taker_id];
            (
                t.store.get(res).copied().unwrap_or(0),
                t.store.get("energy").copied().unwrap_or(0),
            )
        };
        let need_energy = if res == "energy" { amount + cost } else { cost };
        if have < amount || energy < need_energy {
            return;
        }
        {
            let t = world
                .rooms
                .get_mut(&taker_room)
                .unwrap()
                .objects
                .get_mut(&taker_id)
                .unwrap();
            store_add(&mut t.store, res, -(amount as i64));
            store_add(&mut t.store, "energy", -(cost as i64));
            t.cooldown_time = tick + constants::TERMINAL_COOLDOWN;
        }
        world.market_credits += price_total;
    }

    // 扣减订单剩余量
    if let Some(o) = world.market_orders.get_mut(order_id) {
        o.amount -= amount;
    }
    // 成交记录 (getHistory 数据源)
    world.market_deals.push(MarketDeal {
        tick,
        resource_type: res.to_string(),
        amount,
        price: order.price,
    });
    if world.market_deals.len() > MARKET_DEALS_KEEP {
        let drop = world.market_deals.len() - MARKET_DEALS_KEEP;
        world.market_deals.drain(0..drop);
    }
    // 转账记录 (官方 market 成交也进 transactions): 资源从卖方房 → 买方房
    let (from, to, recipient_mine) = if order.order_type == "sell" {
        (order.room_name.clone(), taker_room.clone(), true)
    } else {
        (taker_room.clone(), order.room_name.clone(), false)
    };
    let tx_id = world.alloc_id();
    world.transactions.insert(
        0,
        Transaction {
            id: tx_id,
            time: tick,
            resource_type: res.to_string(),
            amount,
            from,
            to,
            description: None,
            recipient_mine,
        },
    );
    world.transactions.truncate(TRANSACTIONS_KEEP);
}

/// 本玩家终端的 (id, 房间): 指定房间优先, 否则本玩家唯一有终端的房间
pub fn our_terminal_id(world: &World, room_name: &str) -> Option<u64> {
    world
        .rooms
        .get(room_name)
        .and_then(|r| r.objects.values().find(|o| o.obj_type == "terminal" && o.mine))
        .map(|t| t.id)
}

/// 接单方终端所在房间: 指定房间优先; 未指定则要求本玩家恰有一个带终端的房间
fn resolve_taker_room(world: &World, your_room: Option<&str>) -> Option<String> {
    if let Some(r) = your_room {
        return if our_terminal_id(world, r).is_some() { Some(r.to_string()) } else { None };
    }
    let mut rooms: Vec<String> = world
        .rooms
        .iter()
        .filter(|(_, r)| r.objects.values().any(|o| o.obj_type == "terminal" && o.mine))
        .map(|(name, _)| name.clone())
        .collect();
    if rooms.len() == 1 {
        rooms.pop()
    } else {
        None
    }
}

/// 按资源聚合的成交历史 (官方 getHistory 的近似: date 用 tick, 同 tick 合并)
pub fn history_for(world: &World, resource_type: &str) -> Vec<MarketHistoryEntry> {
    let mut by_tick: BTreeMap<u64, (u32, u32, f64, f64)> = BTreeMap::new();
    for d in world.market_deals.iter().filter(|d| d.resource_type == resource_type) {
        let e = by_tick.entry(d.tick).or_insert((0, 0, 0.0, 0.0));
        e.0 += 1; // transactions
        e.1 += d.amount; // volume
        e.2 += d.price * d.amount as f64; // 加权价累加
        e.3 += d.price * d.price * d.amount as f64; // 方差用
    }
    by_tick
        .into_iter()
        .map(|(tick, (txs, volume, weighted, weighted_sq))| {
            let avg = if volume > 0 { weighted / volume as f64 } else { 0.0 };
            let var = if volume > 0 { (weighted_sq / volume as f64) - avg * avg } else { 0.0 };
            MarketHistoryEntry {
                date: tick,
                transactions: txs,
                volume,
                avg_price: avg,
                stddev_price: var.max(0.0).sqrt(),
            }
        })
        .collect()
}

/// getHistory 单条聚合结果
#[derive(Clone, Debug)]
pub struct MarketHistoryEntry {
    pub date: u64,
    pub transactions: u32,
    pub volume: u32,
    pub avg_price: f64,
    pub stddev_price: f64,
}
