# Transfer Window

<!-- transfer-window-current-version -->
当前项目版本：`v0.1.6`

Transfer Window 是一款统一物理事实层上的近未来行星际航运与工业经营游戏。本仓库已完成
[alpha-v0.1「完整太阳系」](./docs/roadmap/alpha-v0.1-完整太阳系.md)，并正在实现
[alpha-v0.2「可达空间」](./docs/roadmap/alpha-v0.2-可达空间.md)；当前已具备可查询、推进和保存的太阳系浏览器，以及首批舰船工程事实层。

## 开发

```bash
cargo test --workspace
npm install
npm run dev
```

完整验收命令见 [alpha-v0.1 验收记录](./docs/acceptance/alpha-v0.1.md)。
版本演进记录见 [更新日志](./CHANGELOG.md)。

版本发布通过 GitHub Actions 的 `version` 工作流完成；本地可运行 `npm run version:check` 检查版本同步状态。
