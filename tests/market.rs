//! 市场机制回归测试: 订单 / 成交 / credits / 终端跨房交易。
//! 语义来源: docs.screeps.com/market.html (下单 5% 手续费; 运输能量费由接单方终端扣除;
//! 资源卖方→买方, credits 买方→卖方; 资源/credits 不足则订单未激活)。
//! 纯引擎驱动 (不经过 V8), 完全确定性。

use screeps_sim::engine::intents::IntentList;
use screeps_sim::engine::market::{self, MarketOrder};
use screeps_sim::engine::movement::Movement;
use screeps_sim::engine::objects::Obj;
use screeps_sim::engine::scenario::{build_world, Scenario};
use screeps_sim::engine::snapshot::{build_snapshot, build_snapshot_value, CpuView, SnapshotOptions};
use screeps_sim::engine::tick;
use screeps_sim::engine::world::World;

fn setup(json: &str) -> (World, Movement) {
    let scenario = Scenario::from_json(json).unwrap();
    (build_world(&scenario).unwrap(), Movement::default())
}

/// 提交一个 market 意图并跑一个 tick
fn market_tick(world: &mut World, movement: &mut Movement, name: &str, data: serde_json::Value) {
    let json = serde_json::json!([{ "o": "market", "n": name, "d": data }]).to_string();
    tick::run_tick(world, movement, &IntentList::parse(&json));
}

fn terminal<'a>(world: &'a World, room: &str) -> &'a Obj {
    world.rooms[room]
        .objects
        .values()
        .find(|o| o.obj_type == "terminal")
        .unwrap_or_else(|| panic!("{room} 无终端"))
}

fn store_of(world: &World, room: &str, res: &str) -> u32 {
    terminal(world, room).store.get(res).copied().unwrap_or(0)
}

fn order_by_id<'a>(world: &'a World, id: &str) -> Option<&'a MarketOrder> {
    world.market_orders.get(id)
}

fn only_order(world: &World) -> &MarketOrder {
    assert_eq!(world.market_orders.len(), 1, "期望恰有一个订单");
    world.market_orders.values().next().unwrap()
}

// ── 场景 ──

/// 基础市场场景: W5N8 有一个 RCL6 控制器 + 终端 (energy 50000);
/// 账户 credits 10000; 无外部订单。
const MARKET_SCENE: &str = r#"{
    "name": "market",
    "seed": 1,
    "market": { "credits": 10000, "orders": [] },
    "rooms": [
        { "name": "W5N8", "terrain": { "kind": "flat" },
          "controller": { "level": 6, "pos": [25, 25] },
          "structures": [{ "type": "terminal", "pos": [20, 20], "store": { "energy": 50000 } }] }
    ]
}"#;

/// 含外部卖出订单 (W7N8 卖 H, 单价 2, 总量 1000; 距 W5N8 两房)
const SELL_ORDER_SCENE: &str = r#"{
    "name": "market-sell",
    "seed": 1,
    "market": { "credits": 10000, "orders": [
        { "type": "sell", "resourceType": "H", "roomName": "W7N8", "price": 2, "totalAmount": 1000 }
    ] },
    "rooms": [
        { "name": "W5N8", "terrain": { "kind": "flat" },
          "controller": { "level": 6, "pos": [25, 25] },
          "structures": [{ "type": "terminal", "pos": [20, 20], "store": { "energy": 50000 } }] }
    ]
}"#;

/// 含外部买入订单 + 我们终端有 H 500 (供卖出)
const BUY_ORDER_SCENE: &str = r#"{
    "name": "market-buy",
    "seed": 1,
    "market": { "credits": 10000, "orders": [
        { "type": "buy", "resourceType": "H", "roomName": "W7N8", "price": 3, "totalAmount": 1000 }
    ] },
    "rooms": [
        { "name": "W5N8", "terrain": { "kind": "flat" },
          "controller": { "level": 6, "pos": [25, 25] },
          "structures": [{ "type": "terminal", "pos": [20, 20], "store": { "energy": 50000, "H": 500 } }] }
    ]
}"#;

// ── 测试 ──

/// 创建订单立即收取「总价 × 5%」手续费 (买单与卖单一致), 订单字段正确
#[test]
fn create_order_charges_five_percent_fee_for_both_types() {
    let (mut world, mut mv) = setup(MARKET_SCENE);
    assert_eq!(world.market_credits, 10000.0);

    market_tick(
        &mut world,
        &mut mv,
        "createOrder",
        serde_json::json!({ "type": "buy", "resourceType": "energy", "roomName": "W5N8", "price": 10, "totalAmount": 100 }),
    );
    // fee = 10 * 100 * 0.05 = 50
    assert!((world.market_credits - 9950.0).abs() < 1e-6, "credits={}", world.market_credits);
    let o = only_order(&world);
    assert_eq!(o.order_type, "buy");
    assert_eq!(o.resource_type, "energy");
    assert_eq!(o.price, 10.0);
    assert_eq!(o.amount, 100);
    assert_eq!(o.total_amount, 100);
    assert!(!o.external, "本玩家订单 external=false");

    // 卖单同样收 5%: 100 * 4 * 0.05 = 20 → 9930
    market_tick(
        &mut world,
        &mut mv,
        "createOrder",
        serde_json::json!({ "type": "sell", "resourceType": "H", "roomName": "W5N8", "price": 4, "totalAmount": 100 }),
    );
    assert!((world.market_credits - 9930.0).abs() < 1e-6, "credits={}", world.market_credits);
    assert_eq!(world.market_orders.len(), 2);
}

/// 买入外部卖单: 资源进我们终端, 我们付 credits, 能量费从我们 (接单方) 终端扣除
#[test]
fn deal_sell_order_buys_resource_and_charges_taker() {
    let (mut world, mut mv) = setup(SELL_ORDER_SCENE);
    let oid = only_order(&world).id.clone();
    assert_eq!(store_of(&world, "W5N8", "H"), 0);

    market_tick(
        &mut world,
        &mut mv,
        "deal",
        serde_json::json!({ "orderId": oid, "amount": 200, "roomName": "W5N8" }),
    );

    // 200 H 进入我们终端; credits 10000 - 200*2 = 9600; 能量费 ceil(200*(1-e^-2/30))=13
    assert_eq!(store_of(&world, "W5N8", "H"), 200);
    assert_eq!(store_of(&world, "W5N8", "energy"), 50000 - 13);
    assert!((world.market_credits - 9600.0).abs() < 1e-6, "credits={}", world.market_credits);
    assert_eq!(order_by_id(&world, &oid).unwrap().amount, 800, "订单剩余量应扣减");
    // 成交后终端进入冷却
    assert!(terminal(&world, "W5N8").cooldown_time > world.tick);
}

/// 卖出外部买单: 资源离开我们终端, 我们收 credits, 能量费仍从我们 (接单方) 终端扣除
#[test]
fn deal_buy_order_sells_resource_and_gains_credits() {
    let (mut world, mut mv) = setup(BUY_ORDER_SCENE);
    let oid = only_order(&world).id.clone();

    market_tick(
        &mut world,
        &mut mv,
        "deal",
        serde_json::json!({ "orderId": oid, "amount": 100, "roomName": "W5N8" }),
    );

    // 100 H 离开我们终端 (500→400); credits 10000 + 100*3 = 10300; 能量费 ceil(100*(1-e^-2/30))=7
    assert_eq!(store_of(&world, "W5N8", "H"), 400);
    assert_eq!(store_of(&world, "W5N8", "energy"), 50000 - 7);
    assert!((world.market_credits - 10300.0).abs() < 1e-6, "credits={}", world.market_credits);
    assert_eq!(order_by_id(&world, &oid).unwrap().amount, 900);
}

/// 卖出能量时, 能量费与卖出量都从同一终端能量里扣 (官方: 接单方出运输费)
#[test]
fn deal_energy_resource_charges_amount_plus_transport() {
    let scene = r#"{
        "name": "market-energy",
        "seed": 1,
        "market": { "credits": 0, "orders": [
            { "type": "buy", "resourceType": "energy", "roomName": "W7N8", "price": 0.5, "totalAmount": 1000 }
        ] },
        "rooms": [
            { "name": "W5N8", "terrain": { "kind": "flat" },
              "controller": { "level": 6, "pos": [25, 25] },
              "structures": [{ "type": "terminal", "pos": [20, 20], "store": { "energy": 5000 } }] }
        ]
    }"#;
    let (mut world, mut mv) = setup(scene);
    let oid = only_order(&world).id.clone();

    market_tick(
        &mut world,
        &mut mv,
        "deal",
        serde_json::json!({ "orderId": oid, "amount": 100, "roomName": "W5N8" }),
    );

    // 卖 100 energy: 终端能量 5000 - 100 - ceil(100*(1-e^-2/30))=7 → 4893
    assert_eq!(store_of(&world, "W5N8", "energy"), 5000 - 100 - 7);
    assert!((world.market_credits - 50.0).abs() < 1e-6, "credits={}", world.market_credits);
}

/// credits 不足时成交被拒 (引擎侧兜底)
#[test]
fn deal_rejected_when_credits_insufficient() {
    let scene = r#"{
        "name": "market-poor",
        "seed": 1,
        "market": { "credits": 10, "orders": [
            { "type": "sell", "resourceType": "H", "roomName": "W7N8", "price": 100, "totalAmount": 1000 }
        ] },
        "rooms": [
            { "name": "W5N8", "terrain": { "kind": "flat" },
              "controller": { "level": 6, "pos": [25, 25] },
              "structures": [{ "type": "terminal", "pos": [20, 20], "store": { "energy": 50000 } }] }
        ]
    }"#;
    let (mut world, mut mv) = setup(scene);
    let oid = only_order(&world).id.clone();

    market_tick(
        &mut world,
        &mut mv,
        "deal",
        serde_json::json!({ "orderId": oid, "amount": 10, "roomName": "W5N8" }),
    );
    // 10 * 100 = 1000 > 10 → 不成交
    assert_eq!(store_of(&world, "W5N8", "H"), 0);
    assert!((world.market_credits - 10.0).abs() < 1e-6);
    assert_eq!(order_by_id(&world, &oid).unwrap().amount, 1000);
}

/// 撤单只对自己的订单生效 (外部订单不受影响); 改价 / 追加数量
#[test]
fn cancel_change_price_and_extend_only_affect_own_orders() {
    let (mut world, mut mv) = setup(MARKET_SCENE);
    market_tick(
        &mut world,
        &mut mv,
        "createOrder",
        serde_json::json!({ "type": "sell", "resourceType": "H", "roomName": "W5N8", "price": 5, "totalAmount": 100 }),
    );
    let oid = only_order(&world).id.clone();

    // 改价 + 追加
    market_tick(
        &mut world,
        &mut mv,
        "changeOrderPrice",
        serde_json::json!({ "orderId": oid, "newPrice": 7.5 }),
    );
    market_tick(
        &mut world,
        &mut mv,
        "extendOrder",
        serde_json::json!({ "orderId": oid, "addAmount": 50 }),
    );
    let o = order_by_id(&world, &oid).unwrap();
    assert_eq!(o.price, 7.5);
    assert_eq!(o.amount, 150);
    assert_eq!(o.total_amount, 150);

    // 撤单
    market_tick(&mut world, &mut mv, "cancelOrder", serde_json::json!({ "orderId": oid }));
    assert!(order_by_id(&world, &oid).is_none(), "自己的订单应被移除");
}

/// 外部订单不会被 cancel / changePrice / extend 影响
#[test]
fn external_orders_are_immutable_to_player_ops() {
    let (mut world, mut mv) = setup(SELL_ORDER_SCENE);
    let oid = only_order(&world).id.clone();

    market_tick(&mut world, &mut mv, "cancelOrder", serde_json::json!({ "orderId": oid }));
    market_tick(
        &mut world,
        &mut mv,
        "changeOrderPrice",
        serde_json::json!({ "orderId": oid, "newPrice": 99.0 }),
    );
    market_tick(
        &mut world,
        &mut mv,
        "extendOrder",
        serde_json::json!({ "orderId": oid, "addAmount": 500 }),
    );
    let o = order_by_id(&world, &oid).unwrap();
    assert_eq!(o.price, 2.0, "外部订单价格不变");
    assert_eq!(o.total_amount, 1000, "外部订单总量不变");
}

/// 订单激活判定: 外部恒激活; 我方买单 credits 不足 / 卖单资源不足 → 未激活
#[test]
fn order_active_reflects_maker_capacity() {
    let (mut world, _mv) = setup(MARKET_SCENE);
    // 外部卖单恒激活
    let ext = MarketOrder {
        id: "x".into(),
        order_type: "sell".into(),
        resource_type: "H".into(),
        room_name: "W5N8".into(),
        price: 1.0,
        amount: 100,
        total_amount: 100,
        created: 0,
        external: true,
    };
    assert!(market::is_active(&world, &ext));

    // 我方买单: credits 10000, 100*10 = 1000 可负担 → 激活; 抬高到 20000 不可负担 → 未激活
    let mut own_buy = ext.clone();
    own_buy.external = false;
    own_buy.order_type = "buy".into();
    own_buy.price = 10.0;
    assert!(market::is_active(&world, &own_buy));
    own_buy.price = 200.0; // 100 * 200 = 20000 > 10000
    assert!(!market::is_active(&world, &own_buy));

    // 我方卖单: 终端无 H → 未激活
    let mut own_sell = ext.clone();
    own_sell.external = false;
    assert!(!market::is_active(&world, &own_sell));
    // 给终端补 H 500 后 → 激活
    world
        .rooms
        .get_mut("W5N8")
        .unwrap()
        .objects
        .values_mut()
        .find(|o| o.obj_type == "terminal")
        .unwrap()
        .store
        .insert("H".into(), 500);
    assert!(market::is_active(&world, &own_sell));
}

/// 成交历史按 tick 聚合 (getHistory 数据源)
#[test]
fn history_aggregates_deals() {
    let (mut world, mut mv) = setup(SELL_ORDER_SCENE);
    let oid = only_order(&world).id.clone();
    market_tick(
        &mut world,
        &mut mv,
        "deal",
        serde_json::json!({ "orderId": oid, "amount": 200, "roomName": "W5N8" }),
    );
    let hist = market::history_for(&world, "H");
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].volume, 200);
    assert_eq!(hist[0].transactions, 1);
    assert!((hist[0].avg_price - 2.0).abs() < 1e-9);
    // 未成交资源无历史
    assert!(market::history_for(&world, "energy").is_empty());
}

/// 快照携带市场字段 (快路径与 Value 路径一致)
#[test]
fn snapshot_exposes_market_fields() {
    let (mut world, mut mv) = setup(SELL_ORDER_SCENE);
    let oid = only_order(&world).id.clone();
    market_tick(
        &mut world,
        &mut mv,
        "deal",
        serde_json::json!({ "orderId": oid, "amount": 200, "roomName": "W5N8" }),
    );
    let opts = SnapshotOptions {
        cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10000.0 },
        always_visible: true,
        include_terrain: true,
    };
    let fast: serde_json::Value = serde_json::from_str(&build_snapshot(&world, &opts)).unwrap();
    let slow = build_snapshot_value(&world, &opts);
    assert!((fast["marketCredits"].as_f64().unwrap() - 9600.0).abs() < 1e-6);
    let orders = fast["marketOrders"].as_array().unwrap();
    assert_eq!(orders.len(), 1);
    let o = &orders[0];
    assert_eq!(o["type"], "sell");
    assert_eq!(o["resourceType"], "H");
    assert_eq!(o["roomName"], "W7N8");
    assert_eq!(o["remainingAmount"], 800);
    assert_eq!(o["totalAmount"], 1000);
    assert_eq!(o["active"], true);
    // Value 路径与快路径一致
    assert_eq!(fast["marketCredits"], slow["marketCredits"]);
    assert_eq!(fast["marketOrders"], slow["marketOrders"]);
    // 成交历史
    assert_eq!(fast["marketHistory"]["H"][0]["volume"], 200);
    assert_eq!(slow["marketHistory"]["H"][0]["volume"], 200);
}

/// 无市场活动的场景仍携带市场字段 (与 transactions 一样恒定输出, 空 history 为 {})
#[test]
fn snapshot_includes_empty_market_history() {
    let (world, _mv) = setup(MARKET_SCENE);
    let opts = SnapshotOptions {
        cpu: CpuView { limit: 100.0, tick_limit: 500.0, bucket: 10000.0 },
        always_visible: true,
        include_terrain: false,
    };
    let fast: serde_json::Value = serde_json::from_str(&build_snapshot(&world, &opts)).unwrap();
    let slow = build_snapshot_value(&world, &opts);
    assert_eq!(fast["marketHistory"], serde_json::json!({}), "无成交 → 空对象");
    assert_eq!(fast["marketOrders"], serde_json::json!([]));
    assert_eq!(fast["marketCredits"], 10000.0);
    assert_eq!(fast["marketHistory"], slow["marketHistory"]);
    assert_eq!(fast["marketOrders"], slow["marketOrders"]);
}

/// 场景校验: 非法订单参数被拒
#[test]
fn scenario_validation_rejects_bad_orders() {
    let bad_type = MARKET_SCENE.replace(
        r#""orders": []"#,
        r#""orders": [ { "type": "hold", "resourceType": "H", "roomName": "W5N8", "price": 1, "totalAmount": 1 } ]"#,
    );
    assert!(Scenario::from_json(&bad_type).is_err(), "非法订单 type 应被拒");

    let bad_price = MARKET_SCENE.replace(
        r#""orders": []"#,
        r#""orders": [ { "type": "buy", "resourceType": "H", "roomName": "W5N8", "price": 0, "totalAmount": 1 } ]"#,
    );
    assert!(Scenario::from_json(&bad_price).is_err(), "price=0 应被拒");
}
