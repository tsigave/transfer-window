# Transfer Window

<!-- transfer-window-current-version -->
当前项目版本：`v0.2.1`

Transfer Window 是一款统一物理事实层上的近未来行星际航运与工业经营游戏。本仓库已完成
[alpha-v0.1「完整太阳系」](./docs/roadmap/alpha-v0.1-完整太阳系.md)与
[alpha-v0.2「可达空间」](./docs/roadmap/alpha-v0.2-可达空间.md)；当前已具备可查询、推进和保存的太阳系浏览器、舰船工程事实层、覆盖全部登记天体的可复现航迹求解、Pareto 规划界面，以及可保存和确定性回放的航行计划执行。

## 开发

```bash
npm install
npm run dev
```

`npm run dev` 同时启动权威 Rust API（`http://127.0.0.1:3000`）和 React 网页（`http://localhost:1420`）。也可以分别运行 `npm run dev:server` 与 `npm run dev:web`。

生产构建：

```bash
cargo build -p sim-server --release
npm run build
```

网页端通过版本化 `/api/v1` 使用完整 Rust 事实层、航迹求解、航行命令和 SQLite 存档，不包含 TypeScript 物理替代实现。部署参数及反向代理要求见[网页前后端技术指导](./docs/07-v0.2-网页前后端分离.md)。Tauri 边界源码暂时保留，但不作为当前发行目标。

完整验收命令见 [v0.2.1 验收记录](./docs/acceptance/v0.2.1.md)。
版本演进记录见 [更新日志](./CHANGELOG.md)。

版本升级通过 GitHub Actions 的 `version` 工作流完成；本地可运行 `npm run version:check` 检查版本同步状态。
