-- 修复权限树中文名称异常显示为问号的问题

SET NAMES utf8mb4;

UPDATE `permission`
SET `permission_name` = CASE `permission_code`
  WHEN 'workspace' THEN '工作台'
  WHEN 'tasks' THEN '任务管理'
  WHEN 'tasks:list' THEN '任务列表'
  WHEN 'tasks:list:create' THEN '新增'
  WHEN 'tasks:list:update' THEN '修改'
  WHEN 'tasks:list:delete' THEN '删除'
  WHEN 'tasks:records' THEN '环节记录管理'
  WHEN 'tasks:records:update' THEN '修改'
  WHEN 'tasks:records:export' THEN '导出'
  WHEN 'tasks:evaluation' THEN '服务质量评估'
  WHEN 'tasks:evaluation:score' THEN '评分'
  WHEN 'tasks:evaluation:update' THEN '修改'
  WHEN 'users' THEN '用户管理'
  WHEN 'users:customers' THEN '客户管理'
  WHEN 'users:customers:create' THEN '新增'
  WHEN 'users:customers:update' THEN '修改'
  WHEN 'users:customers:delete' THEN '删除'
  WHEN 'users:dealers' THEN '经销商管理'
  WHEN 'users:dealers:create' THEN '新增'
  WHEN 'users:dealers:update' THEN '修改'
  WHEN 'users:dealers:delete' THEN '删除'
  WHEN 'users:staff' THEN '师傅管理'
  WHEN 'users:staff:create' THEN '新增'
  WHEN 'users:staff:update' THEN '修改'
  WHEN 'users:staff:delete' THEN '删除'
  WHEN 'settings' THEN '系统设置'
  WHEN 'settings:roles' THEN '角色管理'
  WHEN 'settings:roles:create' THEN '新增'
  WHEN 'settings:roles:update' THEN '修改'
  WHEN 'settings:roles:delete' THEN '删除'
  WHEN 'settings:permissions' THEN '权限管理'
  WHEN 'settings:permissions:create' THEN '新增'
  WHEN 'settings:permissions:update' THEN '修改'
  WHEN 'settings:permissions:delete' THEN '删除'
  WHEN 'settings:privacy-policy' THEN '隐私协议'
  WHEN 'settings:user-agreement' THEN '用户协议'
  WHEN 'master-data' THEN '基础资料'
  WHEN 'master-data:stores' THEN '门店信息'
  WHEN 'master-data:stores:create' THEN '新增'
  WHEN 'master-data:stores:update' THEN '修改'
  WHEN 'master-data:stores:delete' THEN '删除'
  WHEN 'master-data:sources' THEN '客户来源'
  WHEN 'master-data:sources:create' THEN '新增'
  WHEN 'master-data:sources:update' THEN '修改'
  WHEN 'master-data:sources:delete' THEN '删除'
  ELSE `permission_name`
END
WHERE `permission_code` IN (
  'workspace',
  'tasks', 'tasks:list', 'tasks:list:create', 'tasks:list:update', 'tasks:list:delete',
  'tasks:records', 'tasks:records:update', 'tasks:records:export',
  'tasks:evaluation', 'tasks:evaluation:score', 'tasks:evaluation:update',
  'users', 'users:customers', 'users:customers:create', 'users:customers:update', 'users:customers:delete',
  'users:dealers', 'users:dealers:create', 'users:dealers:update', 'users:dealers:delete',
  'users:staff', 'users:staff:create', 'users:staff:update', 'users:staff:delete',
  'settings', 'settings:roles', 'settings:roles:create', 'settings:roles:update', 'settings:roles:delete',
  'settings:permissions', 'settings:permissions:create', 'settings:permissions:update', 'settings:permissions:delete',
  'settings:privacy-policy', 'settings:user-agreement',
  'master-data', 'master-data:stores', 'master-data:stores:create', 'master-data:stores:update', 'master-data:stores:delete',
  'master-data:sources', 'master-data:sources:create', 'master-data:sources:update', 'master-data:sources:delete'
);
