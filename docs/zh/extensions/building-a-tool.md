---
title: 从零构建一个工具
description: 使用 Rust 构建一个天气 WASM 工具
---

本教程带你从零实现一个 weather-tool：通过 Open-Meteo（免费、无需 API Key）获取实时天气、5 天预报与空气质量，并让 IronClaw 代理可直接调用。

目标效果：

> “东京现在天气怎么样？”

完整参考实现：

<Card title="weather-tool source" icon="github" href="https://github.com/matiasbenary/ironclaw/tree/tools/weather/tools-src/weather">
  查看完整代码：lib.rs、Cargo.toml 与 capabilities.json。
</Card>

---

## 前置准备

安装 Rust 并添加 WASM 目标：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-wasip2
```

---

## 1. 创建项目

```bash
cargo new --lib weather-tool
cd weather-tool
```

将 `Cargo.toml` 替换为：

```toml Cargo.toml
[package]
name = "weather-tool"
version = "0.1.0"
edition = "2021"
description = "Weather information tool for IronClaw (WASM component)"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "=0.36"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1

[workspace]
```

<Note>
`cdylib` 是构建 WASM 组件所需产物类型；`[workspace]` 可避免被父工作区自动并入。
</Note>

---

## 2. 接入 WIT 接口

IronClaw 工具是实现了 WIT 接口的 WASM 组件。宿主提供 HTTP、日志与工作区能力；你的工具需导出 `execute`、`schema`、`description`。

`src/lib.rs` 骨架：

```rust src/lib.rs
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::{Deserialize, Serialize};

struct WeatherTool;

impl exports::near::agent::tool::Guest for WeatherTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response { output: Some(result), error: None },
            Err(e) => exports::near::agent::tool::Response { output: None, error: Some(e) },
        }
    }

    fn schema() -> String { SCHEMA.to_string() }

    fn description() -> String {
        "Get weather information using Open-Meteo...".to_string()
    }
}

export!(WeatherTool);
```

---

## 3. 解析参数并分发动作

```rust src/lib.rs
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    GetCurrent(WeatherParams),
    GetForecast(WeatherParams),
    GetAirQuality(AirQualityParams),
}

#[derive(Debug, Deserialize)]
struct WeatherParams {
    city: String,
    #[serde(default)]
    country_code: Option<String>,
    #[serde(default)]
    units: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AirQualityParams {
    lat: f64,
    lon: f64,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let action: Action = serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;
    match action {
        Action::GetCurrent(p) => get_current(p),
        Action::GetForecast(p) => get_forecast(p),
        Action::GetAirQuality(p) => get_air_quality(p),
    }
}
```

<Note>
Rust 侧参数结构必须与 JSON Schema 保持一致，否则模型会构造错误参数。
</Note>

---

## 4. 实现业务动作

实现 `get_current`、`get_forecast`、`get_air_quality`，并调用 Open-Meteo API。

建议拆分两个辅助函数：

- `geocode(city, country_code)`：城市名转经纬度
- `api_get(url)`：统一 HTTP 请求与错误处理

如果 API 需要密钥，不要在 Rust 代码里手工拼接敏感值。应在 capabilities 文件声明，由宿主代理在请求时注入。

---

## 5. 定义 JSON Schema

`schema()` 返回模型可读的参数模式，必须覆盖：

- action 枚举
- 每个 action 的参数字段与类型
- 必填字段
- 可选字段约束

这一步决定模型能否正确调用你的工具。

---

## 6. 添加 capabilities.json

最小示例：

```json
{
  "name": "weather-tool",
  "version": "0.1.0",
  "description": "Weather information tool",
  "network": {
    "allowed_hosts": [
      "geocoding-api.open-meteo.com",
      "api.open-meteo.com"
    ]
  }
}
```

若使用凭据，还应在 `credentials` 中声明注入规则。

---

## 7. 构建为 WASM

```bash
cargo build --release --target wasm32-wasip2
```

产物一般位于：

- `target/wasm32-wasip2/release/weather_tool.wasm`

---

## 8. 安装并测试

```bash
ironclaw tool install ./target/wasm32-wasip2/release/weather_tool.wasm
ironclaw tool list
```

然后在聊天中测试：

- “帮我查东京当前天气”
- “给我看上海未来 5 天预报”
- “查询北京空气质量”

---

## 9. 调试建议

- 先用固定参数本地验证 JSON 解析
- 记录关键日志（城市名、坐标、HTTP 状态）
- 对第三方 API 响应做健壮兜底（字段缺失、空数组、429）
- 输出错误信息时给出可操作建议

---

## 10. 进一步扩展

- 支持多语言输出与单位自动转换
- 增加重试与退避策略
- 引入缓存（按城市与时间窗口）
- 增加天气告警和极端天气提示

<Note>
本页为中文精简版流程，覆盖从 0 到可运行工具的关键步骤。需要逐行完整实现可参考英文原始教程与仓库示例代码。
</Note>
