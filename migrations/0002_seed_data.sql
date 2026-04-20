-- 四专检核系统种子数据
-- 默认管理员账号:
-- 手机号: 13800000000
-- 密码: Admin@123456

SET NAMES utf8mb4;

INSERT INTO `system_version` (`version_no`, `version_title`, `changelog`, `published_at`)
SELECT '0.1.0', '初始化版本', '系统初始化种子数据', NOW()
WHERE NOT EXISTS (
  SELECT 1 FROM `system_version` WHERE `version_no` = '0.1.0'
);

INSERT INTO `store` (`store_name`, `contact_name`, `contact_mobile`, `province`, `city`, `district`, `address`, `status`)
SELECT '杭州西湖演示门店', '演示店长', '13800001111', '浙江省', '杭州市', '西湖区', '文一路123号', 'enabled'
WHERE NOT EXISTS (
  SELECT 1 FROM `store` WHERE `store_name` = '杭州西湖演示门店'
);

INSERT INTO `service_standard` (`service_type`, `title`, `content`, `status`, `version_no`)
SELECT 'install', '安装检核照片标准', '1. 照片中能清晰识别出以下工具: 安装工具箱、冲击钻及钻头、螺丝刀等工具、铅笔、膨胀管/螺丝、备用调节块、滑轮、防尘罩、油漆毛刷、防护大拖鞋。2. 客户房间背景，务必水印和机拍照片，包含时间、地点、门店等信息。', 'enabled', 'v1'
WHERE NOT EXISTS (
  SELECT 1 FROM `service_standard` WHERE `service_type` = 'install' AND `version_no` = 'v1'
);

INSERT INTO `service_standard` (`service_type`, `title`, `content`, `status`, `version_no`)
SELECT 'measure', '量尺照片标准', '量尺现场必须包含窗户位置、量尺工具、客户房间整体环境，照片自动加时间地点水印。', 'enabled', 'v1'
WHERE NOT EXISTS (
  SELECT 1 FROM `service_standard` WHERE `service_type` = 'measure' AND `version_no` = 'v1'
);

INSERT INTO `service_standard` (`service_type`, `title`, `content`, `status`, `version_no`)
SELECT 'ironing', '熨烫照片标准', '熨烫作业需拍摄窗帘整体褶皱前后对比，并包含客户现场环境。', 'enabled', 'v1'
WHERE NOT EXISTS (
  SELECT 1 FROM `service_standard` WHERE `service_type` = 'ironing' AND `version_no` = 'v1'
);

INSERT INTO `service_standard` (`service_type`, `title`, `content`, `status`, `version_no`)
SELECT 'cleaning', '除尘除螨照片标准', '清洗设备、施工现场、客户房间背景需同时入镜，照片带门店与时间水印。', 'enabled', 'v1'
WHERE NOT EXISTS (
  SELECT 1 FROM `service_standard` WHERE `service_type` = 'cleaning' AND `version_no` = 'v1'
);

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'module.dashboard', '工作台', 'menu', NULL, 1
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'module.dashboard');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'module.tasks', '任务管理', 'menu', NULL, 2
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'module.tasks');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'module.customers', '客户管理', 'menu', NULL, 3
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'module.customers');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'module.technicians', '技师管理', 'menu', NULL, 4
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'module.technicians');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'module.users', '用户管理', 'menu', NULL, 5
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'module.users');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'module.system', '系统设置', 'menu', NULL, 6
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'module.system');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'module.reports', '报表统计', 'menu', NULL, 7
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'module.reports');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'module.oms', 'OMS对接', 'menu', NULL, 8
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'module.oms');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'task.create', '创建任务', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'module.tasks'), 101
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'task.create');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'task.assign', '分派任务', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'module.tasks'), 102
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'task.assign');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'task.review', '任务评价', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'module.tasks'), 103
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'task.review');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'customer.manage', '客户维护', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'module.customers'), 201
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'customer.manage');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'technician.manage', '技师维护', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'module.technicians'), 301
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'technician.manage');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'user.manage', '用户维护', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'module.users'), 401
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'user.manage');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'system.manage', '系统配置维护', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'module.system'), 501
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'system.manage');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'report.view', '查看报表', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'module.reports'), 601
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'report.view');

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'oms.manage', 'OMS配置维护', 'action', (SELECT `id` FROM `permission` WHERE `permission_code` = 'module.oms'), 701
WHERE NOT EXISTS (SELECT 1 FROM `permission` WHERE `permission_code` = 'oms.manage');

INSERT INTO `user` (`mobile`, `password_hash`, `name`, `email`, `status`)
SELECT '13800000000', '$2b$10$0pAAaLAPE6GcIT7m4yK6muqLff6fnoBNdkyoW/muQ8wHduSi7HP5.', '系统管理员', 'admin@jinguanjia.local', 'enabled'
WHERE NOT EXISTS (
  SELECT 1 FROM `user` WHERE `mobile` = '13800000000'
);

INSERT INTO `user_role` (`user_id`, `role_id`)
SELECT u.`id`, r.`id`
FROM `user` u
JOIN `role` r ON r.`role_code` = 'super_admin'
WHERE u.`mobile` = '13800000000'
  AND NOT EXISTS (
    SELECT 1 FROM `user_role` ur WHERE ur.`user_id` = u.`id` AND ur.`role_id` = r.`id`
  );

INSERT INTO `role_permission` (`role_id`, `permission_id`)
SELECT r.`id`, p.`id`
FROM `role` r
JOIN `permission` p ON 1 = 1
WHERE r.`role_code` = 'super_admin'
  AND NOT EXISTS (
    SELECT 1 FROM `role_permission` rp WHERE rp.`role_id` = r.`id` AND rp.`permission_id` = p.`id`
  );

INSERT INTO `user_message_setting` (`user_id`, `new_task_enabled`, `review_reminder_enabled`, `sms_enabled`, `wechat_enabled`)
SELECT u.`id`, 1, 1, 1, 1
FROM `user` u
WHERE u.`mobile` = '13800000000'
  AND NOT EXISTS (
    SELECT 1 FROM `user_message_setting` ums WHERE ums.`user_id` = u.`id`
  );
