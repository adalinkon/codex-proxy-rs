<!-- prettier-ignore -->
<div align="center">

<img src="frontend/public/favicon.svg" alt="Codex Proxy RS" width="80" height="80" />

# Codex Proxy RS

面向 Codex 的自托管多账号 AI 网关

[![CI](https://github.com/zyycn/codex-proxy-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/zyycn/codex-proxy-rs/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/zyycn/codex-proxy-rs?display_name=tag&sort=semver&style=flat-square)](https://github.com/zyycn/codex-proxy-rs/releases)
[![GHCR](https://img.shields.io/badge/GHCR-codex--proxy--rs-2496ED?logo=docker&logoColor=white&style=flat-square)](https://github.com/zyycn/codex-proxy-rs/pkgs/container/codex-proxy-rs)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg?style=flat-square)](LICENSE)

[快速开始](#快速开始) · [客户端接入](#客户端接入) · [文档](#文档) · [社区](#社区) · [许可证](#许可证)

</div>

> [!NOTE]
> 本项目提供 Responses API，不支持 `/v1/chat/completions`。接入前请确认客户端支持 Responses 协议。

## 快速开始

使用 Docker Compose 部署发布镜像 `ghcr.io/zyycn/codex-proxy-rs:latest`，同时启动 PostgreSQL 和 Redis。
以下命令适用于 Linux amd64/arm64，需要 Docker Engine、Docker Compose Plugin、curl 和 OpenSSL。已有部署请先看
[升级说明](deploy/README.md#镜像升级与源码构建)，不要覆盖原配置。

### 1. 下载部署文件并配置

```bash
mkdir -p codex-proxy-rs/deploy && cd codex-proxy-rs

curl -fsSL https://raw.githubusercontent.com/zyycn/codex-proxy-rs/main/deploy/compose.yaml \
  -o deploy/compose.yaml
curl -fsSL https://raw.githubusercontent.com/zyycn/codex-proxy-rs/main/deploy/config.example.yaml \
  -o deploy/config.example.yaml

install -d -m 0750 .runtime/postgres .runtime/redis
sudo install -d -m 0770 -o "$(id -u)" -g 10001 .runtime/data .runtime/logs
sudo install -m 0640 -o "$(id -u)" -g 10001 deploy/config.example.yaml deploy/config.yaml
```

分别生成数据库和 Redis 密码：

```bash
openssl rand -hex 24
openssl rand -hex 24
```

编辑 `deploy/config.yaml`，填好以下三项：

| 配置项 | 填写内容 |
| --- | --- |
| `store.database.password` | 第一个生成的 48 位十六进制密码 |
| `store.redis.password` | 第二个生成的 48 位十六进制密码 |
| `admin.default_password` | 管理员初始密码，至少 12 位，不能包含 `$` |

### 2. 启动服务

```bash
docker compose -f deploy/compose.yaml config --quiet
docker compose -f deploy/compose.yaml pull
docker compose -f deploy/compose.yaml up -d --no-build --wait
curl -i http://127.0.0.1:8080/healthz
```

健康检查返回 `204 No Content` 后，打开 `http://127.0.0.1:8080`，
使用 `admin@cpr.local` 和刚设置的管理员密码登录。

默认地址只能在服务器本机访问。从其他设备使用时，需要配置
[HTTPS 反向代理](deploy/README.md#公网访问)。

### 3. 添加账号与客户端密钥

1. 在「账号」中添加账号，完成授权或导入。
2. 按需建立账号分组，再创建客户端密钥并选择可用分组。**不选分组表示继承所属用户的授权**，初始管理员默认可使用全部账号。
3. 打开密钥的「使用密钥」，复制客户端配置。

### 用户与个人面板

管理员可在「用户管理」中创建普通用户、分配可用账号分组，并设置用户共享的日／七天美元限额、并发与 RPM。
管理员与普通用户共用登录页面；不开放自助注册。普通用户可查看自己的额度、创建与管理自己的 Key、
查看使用记录及修改密码。管理员侧栏下方也提供相同的个人入口。
「个人资料」合并基本资料、日／周额度、共享请求限制、可用分组与修改密码入口，采用单列卡片布局。
普通用户登录后默认进入 `/me/profile`，旧 `/me/overview` 和 `/me/settings` 地址自动跳转。
个人 API 密钥和使用统计复用管理员页面的表格、筛选、分页、额度和图表，只读取自己的数据；Key 名称与
标签可修改，授权及限额由管理员设置。上游账号身份和原始运维诊断仅管理员可见。

普通用户未分配分组时无法调用模型；新建 Key 继承用户授权，同一用户的所有 Key 共享用户额度。
日额度每天北京时间零点重置，周额度从用户或 Key 各自创建当天零点起每七天连续推进，闲置和停用不改变周期。
管理员可在用户管理中重置用户日／周已用，周周期从操作当天重新起算；Key 附加额度和历史费用保留。
Key 自身已有的限制仍同时生效。用户停用后会话失效，所有所属 Key 拒绝新请求；密码修改或重置会使
该用户的既有会话失效。详情见 [用户与额度 API](docs/api.md#用户与个人面板)。

## 客户端接入

**Codex CLI / 桌面端**：在「使用密钥」中按操作系统复制配置，或通过 CCSwitch 导入。
合并到客户端配置后重启 Codex。完整步骤、生图配置与排障见[客户端配置](deploy/README.md#客户端配置)。

**其他 Responses API 客户端**：填写以下信息。

| 配置 | 值 |
| --- | --- |
| Base URL | `http://127.0.0.1:8080/v1`；远程接入使用服务器的 HTTPS 地址 |
| API Key | 管理端创建的客户端密钥 |

可用模型以该密钥查询到的模型列表为准：

```bash
curl http://127.0.0.1:8080/v1/models \
  -H 'Authorization: Bearer <client-api-key>'
```

## 文档

- [客户端接入与生图](deploy/README.md#客户端配置)
- [部署、备份与恢复](deploy/README.md)
- [API 参考](docs/api.md)
- [系统架构](docs/architecture.md)
- [管理端主题](docs/theme.md)
- [数据库迁移](backend/migrations/README.md)
- [贡献与审查](CONTRIBUTING.md)

## 社区

感谢 [LINUX DO](https://linux.do) 社区提供开放、友善的技术交流平台。

## 许可证

本项目基于 [Apache License 2.0](LICENSE) 开源。
