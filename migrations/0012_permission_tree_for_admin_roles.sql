SET NAMES utf8mb4;

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'workspace', '工作台', 'menu', NULL, 1
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'workspace');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks', '任务管理', 'menu', NULL, 2
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:list', '任务列表', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks'), 201
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:list');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:list:create', '新增', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks:list'), 20101
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:list:create');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:list:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks:list'), 20102
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:list:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:list:delete', '删除', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks:list'), 20103
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:list:delete');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:records', '环节记录管理', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks'), 202
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:records');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:records:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks:records'), 20201
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:records:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:records:export', '导出', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks:records'), 20202
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:records:export');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:evaluation', '服务质量评估', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks'), 203
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:evaluation');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:evaluation:score', '评分', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks:evaluation'), 20301
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:evaluation:score');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'tasks:evaluation:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'tasks:evaluation'), 20302
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'tasks:evaluation:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users', '用户管理', 'menu', NULL, 3
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:customers', '客户管理', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users'), 301
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:customers');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:customers:create', '新增', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users:customers'), 30101
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:customers:create');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:customers:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users:customers'), 30102
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:customers:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:customers:delete', '删除', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users:customers'), 30103
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:customers:delete');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:dealers', '经销商管理', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users'), 302
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:dealers');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:dealers:create', '新增', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users:dealers'), 30201
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:dealers:create');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:dealers:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users:dealers'), 30202
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:dealers:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:dealers:delete', '删除', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users:dealers'), 30203
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:dealers:delete');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:staff', '师傅管理', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users'), 303
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:staff');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:staff:create', '新增', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users:staff'), 30301
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:staff:create');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:staff:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users:staff'), 30302
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:staff:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'users:staff:delete', '删除', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'users:staff'), 30303
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'users:staff:delete');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings', '系统设置', 'menu', NULL, 4
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:roles', '角色管理', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings'), 401
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:roles');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:roles:create', '新增', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings:roles'), 40101
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:roles:create');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:roles:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings:roles'), 40102
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:roles:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:roles:delete', '删除', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings:roles'), 40103
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:roles:delete');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:permissions', '权限管理', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings'), 402
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:permissions');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:permissions:create', '新增', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings:permissions'), 40201
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:permissions:create');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:permissions:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings:permissions'), 40202
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:permissions:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:permissions:delete', '删除', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings:permissions'), 40203
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:permissions:delete');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:privacy-policy', '隐私协议', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings'), 403
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:privacy-policy');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:user-agreement', '用户协议', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings'), 404
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:user-agreement');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'master-data', '基础资料', 'menu', NULL, 5
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'master-data');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'master-data:stores', '门店信息', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'master-data'), 501
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'master-data:stores');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'master-data:stores:create', '新增', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'master-data:stores'), 50101
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'master-data:stores:create');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'master-data:stores:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'master-data:stores'), 50102
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'master-data:stores:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'master-data:stores:delete', '删除', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'master-data:stores'), 50103
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'master-data:stores:delete');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'master-data:sources', '客户来源', 'menu', (SELECT `id` FROM `permission` WHERE `permission_code` = 'master-data'), 502
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'master-data:sources');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'master-data:sources:create', '新增', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'master-data:sources'), 50201
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'master-data:sources:create');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'master-data:sources:update', '修改', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'master-data:sources'), 50202
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'master-data:sources:update');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'master-data:sources:delete', '删除', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'master-data:sources'), 50203
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'master-data:sources:delete');

INSERT INTO `role_permission` (`role_id`, `permission_id`)
SELECT r.`id`, p.`id`
FROM `role` r
JOIN `permission` p ON p.`permission_code` IN (
  'workspace','tasks','tasks:list','tasks:list:create','tasks:list:update','tasks:list:delete',
  'tasks:records','tasks:records:update','tasks:records:export',
  'tasks:evaluation','tasks:evaluation:score','tasks:evaluation:update',
  'users','users:customers','users:customers:create','users:customers:update','users:customers:delete',
  'users:dealers','users:dealers:create','users:dealers:update','users:dealers:delete',
  'users:staff','users:staff:create','users:staff:update','users:staff:delete',
  'settings','settings:roles','settings:roles:create','settings:roles:update','settings:roles:delete',
  'settings:permissions','settings:permissions:create','settings:permissions:update','settings:permissions:delete',
  'settings:privacy-policy','settings:user-agreement',
  'master-data','master-data:stores','master-data:stores:create','master-data:stores:update','master-data:stores:delete',
  'master-data:sources','master-data:sources:create','master-data:sources:update','master-data:sources:delete'
)
WHERE r.`role_code` = 'super_admin'
  AND NOT EXISTS (
    SELECT 1 FROM `role_permission` rp WHERE rp.`role_id` = r.`id` AND rp.`permission_id` = p.`id`
  );
