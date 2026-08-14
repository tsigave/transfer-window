# alpha-v0.2 验收记录

本记录对应 [alpha-v0.2「可达空间」Checklist](../roadmap/alpha-v0.2-可达空间.md)，用于证明版本退出条件和发布门禁。所有数值均来自同一仓库工作树的可重放命令。

验收日期：2026-08-13（America/Los_Angeles）

## 自动验证

共同验证命令全部通过：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `npm run lint`、`npm run test`、`npm run build`、`npm run docs:check`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `npm run tauri build`，生成 macOS 应用包与安装介质
- `npm run version:check`

版本专项结果：

| 验证 | 结果 |
| --- | --- |
| 舰船工程审计 | 四套虚构蓝图通过模式、单位、容量、功率、热和寿命校验 |
| 航迹金标准 | Lambert 速度误差 `0.000290918 m/s`；patched-conics 逃逸 Δv `3602.145141 m/s`；有限推力独立复核位置误差 `2.381230 m` |
| 全目录冒烟 | 41 个登记天体全部进入统一入口；15 个产生可执行方案，26 个返回结构化不可行原因 |
| 性能预算 | 地月 `3.735 ms`、地火 `4.586 ms`；均产生代表方案且低于 5 秒预算 |
| 取消清理 | 预取消搜索求值数为 0、残留方案数为 0 |
| 确定性航行 | 倍率 1、100、10,000 的工质、寿命和最终状态哈希完全一致 |
| 最终状态哈希 | `606b6410fd0b6209b0d1ae3f43d7b355d8400ff0c74ee1c87f803fb863798fa2` |
| 存档迁移 | schema 1 代表存档自动生成 `.pre-v0.2.bak` 备份并迁移到 schema 2；航行中保存载入与不中断执行得到相同抵达哈希 |
| 桌面发行包 | 生成 `Transfer Window.app` 与 27 MB 的 `Transfer Window_0.2.0_aarch64.dmg`；DMG 完整性校验通过，SHA-256 为 `83f01af1101c390b19f238627f00dda842eb72f0f0422b0764b3ef76188c2466` |

性能数据来自本机开发构建，只用于验证 5 秒门槛，不作为跨硬件基准承诺。

## 版本验收剧本

1. 同一艘 `Lunar Courier` 在三个地月出发日、五档航程上进行 15 点搜索；测试确认三个出发日均有可执行候选，热图、Pareto 前沿与最快、平衡、节能代表解可展开查看。
2. 可执行方案经 `QuoteTransfer`、`ValidateTransfer` 和 `ScheduleVoyage` 进入事件队列；抵达时计划与实际的工质、寿命和状态误差在批准容差内，三档时间倍率得到同一哈希。另有回归测试确认超容差时保留实际状态并生成诊断，不把舰船强行吸附到目标。
3. 木卫四与海卫一均可从同一目标选择器提交；求解器返回可执行方案或结构化不可行原因，界面始终明确显示“无市场 · 无补给 · 无维修”。
4. 报价后修改载荷再提交旧解，命令以 `SOLUTION_INVALIDATED` 拒绝；世界哈希、资源和事件队列均不变化，可使用新输入重新求解。

React 交互测试覆盖规划器开启、木卫四/海卫一目标选择、三出发日热图、代表解和工程明细展开。Tauri 桌面边界通过编译检查。当前自动化环境无法取得可控桌面窗口，因此没有新增桌面截图；这不影响权威 Rust 命令路径和组件交互门禁，未发现阻断级缺陷。

本地 macOS 包使用 ad-hoc 签名，未使用 Apple Developer ID 公证。面向 Gatekeeper 的公开分发仍需在持有发布凭据的环境中签名并公证；这属于发布渠道操作，仓库不保存相关凭据。

## 发布结论

alpha-v0.2 的四个工作流、专项命令、共同验证命令、存档兼容和四项验收剧本均通过。版本可以进入 `v0.2.0` 发布准备；本记录不代表已创建 Git tag、推送远端或发布安装包。
