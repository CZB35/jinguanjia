## 适用范围

本说明只覆盖后台账号字段从“手机号兼作账号”迁移为“独立账号字段”的变更。

## 1. 背景

当前系统要区分：

- `account`：后台登录账号
- `mobile`：管理员联系电话

经销商、师傅等业务端账号仍继续使用手机号体系，不受本次变更影响。

## 2. 数据库迁移

新增迁移文件：

```text
migrations/0020_admin_account_field.sql
```

### 迁移内容

1. 给 `user` 表新增字段：

```text
account VARCHAR(50)
```

2. 给 `account` 建唯一索引：

```text
uk_user_account
```

3. 回填旧数据：

```text
account = mobile
```

也就是说，迁移完成后，旧后台账号先默认仍可沿用原手机号作为账号值。

## 3. 后端口径

### 后台登录

接口：

```http
POST /auth/login/admin
```

说明：

- 优先按 `account` 查后台账号
- 同时兼容 `username`
- 仅后台角色可登录

### 后台账号管理

接口：

- `GET /users`
- `POST /users`
- `GET /users/{id}`
- `PATCH /users/{id}`

字段口径：

- `account`：后台登录账号
- `mobile`：管理员电话
- `name`：姓名

## 4. 前端影响

### 后台登录页

页面文案改为：

- `账号`
- `密码`

请求示例：

```json
{
  "account": "zhangsan",
  "password": "123456"
}
```

### 后台账号新增 / 编辑页

需要新增字段：

- `account`

原来的 `mobile` 保留，但文案要变成：

- `管理员电话`

### 列表页

字段映射：

- “姓名” -> `name`
- “账号” -> `account`
- “管理员电话” -> `mobile`

## 5. 上线注意事项

上线这批后台账号相关改动前，请先执行迁移：

```text
migrations/0020_admin_account_field.sql
```

否则会出现：

- 列表 `account` 为空
- 后台账号新增 / 编辑异常
- 后台登录按账号查询失败
