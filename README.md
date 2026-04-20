# kgj-backend

技术选型：

1. Web 框架：`axum`
2. ORM：`rbatis`
3. MySQL 驱动：`rbdc-mysql`
4. 认证：`JWT`

启动前：

1. 复制 `.env.example` 为 `.env`
2. 修改 `DATABASE_URL`
3. 修改 `JWT_SECRET`
4. 确认 MySQL 库已创建

本地启动：

```bash
cargo run
```

健康检查：

```bash
GET /api/health
```

已实现认证接口：

1. `POST /api/auth/login/password`
2. `POST /api/auth/refresh`
3. `POST /api/auth/logout`
4. `GET /api/auth/me`
