# 弦电子书同步器

> 在 AstroBox 中同步弦电子书：分章、封面、书籍信息、手环状态与阅读设置。

一个运行于 AstroBox v2 的插件（API Level 3，需要 AstroBox 2.0.0 及以上）

---

## 开发

`wit/` 是指向 [AstroBox-Plugin-WIT](https://github.com/AstralSightStudios/AstroBox-Plugin-WIT) 的
子模块，升级接口定义时先更新它：

```bash
./update_submodules.sh   # Windows: update_submodules.bat
```

构建（首次请先 `rustup target add wasm32-wasip2`）：

```bash
python scripts/build_dist.py --release --package
```

## 功能特性

- **分章同步**：将电子书按章节切分并逐章下发到手环快应用。
- **封面与书籍信息**：同步书籍封面、标题、作者等元信息。
- **手环状态**：查询并显示已连接手环设备状态。
- **阅读设置**：在阅读端调整并回传阅读偏好。

## 许可证

见仓库根目录 `LICENSE`。
