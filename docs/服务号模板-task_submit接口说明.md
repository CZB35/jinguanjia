## 适用范围

本文档只说明服务号模板 `task_submit` 的触发、配置和字段映射。

## 1. 模板用途

模板名称：

```text
客户服务已完成通知
```

模板 ID：

```text
qpT3JJoLU27m2kRNTcO_c6xqBwynxWk862kwfAipBqo
```

适用业务：

```text
task_submit
```

## 2. 触发时机

触发接口：

```http
POST /api/tasks/{id}/submit
```

当技师提交检核成功，任务进入：

```text
pending_dealer_review
```

后端会自动给该任务对应的经销商发送一条服务号模板消息。

## 3. 必要前提

要成功收到服务号通知，必须同时满足：

1. 经销商账号已绑定服务号
2. `wechat_openid` 不为空
3. `wechat_subscribe = 1`
4. 服务号 `appid / secret` 已配置
5. 微信 IP 白名单已配置
6. 已配置 `task_submit` 模板 ID

## 4. 配置项

数据库 `system_config` 中需配置：

```text
wechat.service_account.appid
wechat.service_account.secret
wechat.service_account.bind_redirect_uri
wechat.notify.enabled
wechat.template.task_submit.template_id
```

可选：

```text
wechat.template.task_submit.page_path
wechat.template.task_submit.miniprogram_appid
wechat.template.task_submit.miniprogram_state
```

## 5. 模板字段映射

该模板字段：

- `thing2`：服务人员
- `thing4`：项目名称
- `time12`：完成时间

当前后端映射规则：

- `thing2` -> 师傅姓名
- `thing4` -> 项目名称
- `time12` -> 提交完成时间

## 6. 后端发送内容示例

```json
{
  "thing2": { "value": "李四" },
  "thing4": { "value": "中岳丽景湾" },
  "time12": { "value": "2026-06-12 10:30:00" }
}
```

## 7. 测试方式

### 真实业务测试

1. 让技师开始任务
2. 上传实据
3. 调用：

```http
POST /api/tasks/{id}/submit
```

4. 如果经销商已绑定服务号，则自动收到 `task_submit` 服务号消息。

### 脚本测试

也可以直接用已绑定的经销商 `wechat_openid` 调微信模板消息接口，验证模板字段映射是否正确。

## 8. 当前结论

后端已经支持：

- `task_submit` 模板配置读取
- 自动触发消息
- `thing2 / thing4 / time12` 字段映射
