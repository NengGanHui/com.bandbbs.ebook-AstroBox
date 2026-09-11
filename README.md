# 弦电子书同步器

> 在 AstroBox 中同步弦电子书：分章、封面、书籍信息、手环状态与阅读设置。

一个运行于 AstroBox v2 的 Rust → WebAssembly Component 插件，通过 `interconnect` 通道把电子书内容（分章、封面、书籍信息、手环状态、阅读设置）推送到绑定的小米手环快应用，实现「手机端选书 → 手环端阅读」的同步体验。

---

## 功能特性

- **分章同步**：将电子书按章节切分并逐章下发到手环快应用。
- **封面与书籍信息**：同步书籍封面、标题、作者等元信息。
- **手环状态**：查询并显示已连接手环设备状态。
- **阅读设置**：在阅读端调整并回传阅读偏好。
- **互联消息收发**：通过 `interconnect` 与手环端快应用双向通信（`register_interconnect_recv` 监听、`send-qaic-message` 发送）。

---

## 目录结构

```
电子书同步器/
├── Cargo.toml              # crate-type = ["cdylib"]，依赖 wit-bindgen(async,async-spawn)/waki/serde_json
├── .cargo/config.toml      # [build] target = "wasm32-wasip2"
├── manifest.json           # 插件清单（名称/版本/权限/入口）
├── wit/
│   ├── main.wit            # 世界定义 psys-world
│   └── deps/               # astrobox:psys-host / psys-plugin WIT 接口
├── src/
│   ├── lib.rs              # 生命周期与事件入口
│   ├── ui.rs               # 主界面渲染与 UI 事件处理
│   ├── chapters.rs         # 分章逻辑（含单元测试）
│   ├── protocol.rs         # 协议编解码（含单元测试）
│   └── logger.rs           # tracing 日志初始化
├── scripts/
│   └── build_dist.py       # 官方打包脚本（生成 .abp）
├── dist/                   # 构建产物（wasm + manifest + icon + .abp）
└── rel/                    # 已发布件（ebooksync-ng.wasm + manifest.json + icon.png）
```

---

## 技术规格

| 项目 | 值 |
|---|---|
| 插件名称 | 弦电子书同步器 |
| 版本 | 2.2.0 |
| API Level | `2` |
| WASI Version | `2`（目标 `wasm32-wasip2` WebAssembly Component） |
| 入口 | `ebooksync-ng.wasm` |
| 世界（World） | `psys-world` |

---

## 权限清单

与 `manifest.json` 严格一致，共 4 项，分别对应 WIT 中导入的 host 接口：

| 权限 | WIT 接口 | 用途 |
|---|---|---|
| `device` | `astrobox:psys-host/device` | 获取 / 管理已连接设备列表 |
| `interconnect` | `astrobox:psys-host/interconnect` | 向手环快应用发送 qaic 消息 |
| `register_interconnect_recv` | `astrobox:psys-host/register` | 按包名注册互联消息接收 |
| `thirdpartyapp` | `astrobox:psys-host/thirdpartyapp` | 启动 / 列举手环端第三方快应用 |

---

## 构建与打包

### 前置条件

```bash
rustup target add wasm32-wasip2
# 需要 python3 以运行打包脚本
```

### 编译

```bash
cargo build --release
# 产物：target/wasm32-wasip2/release/ebooksync-ng.wasm
```

### 打包为 .abp

```bash
python scripts/build_dist.py --release --package
# 产出 dist/ 目录及 dist/<插件名>.abp（zip：wasm + manifest.json + icon.png）
```

- `.abp` 文件名由 `manifest.json` 的 `name` 字段派生（`build_dist.py` 的 `make_package_name`）。
- 改名后旧 `.abp` 不会自动删除，需手动清理 `dist/`，否则装机时易装错旧包。
- 打包脚本只读 `name / entry / icon / additional_files`，修改 `website / author / description` 不影响打包结果。

---

## 已知问题 / 限制

- **单元测试本地不可直接运行**：见上文「测试」一节（宿主 target 限制）。
- **开发环境噪音**：`wit/main.wit` 第 3 行有注释提示某编辑器对 WIT 误报，属 IDE 误报，不影响构建。
- **目标平台**：`lib.rs::on_load` 日志显示当前目标为「弦电子书 plus」，部署前需确认手环端快应用版本匹配，避免「版本不兼容 / 手机端版本过低」类问题。

---

## 许可证

见仓库根目录 `LICENSE`。
