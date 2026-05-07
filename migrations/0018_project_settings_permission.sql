-- 新增系统设置下的项目设置菜单权限

SET NAMES utf8mb4;

INSERT INTO `permission` (`permission_code`, `permission_name`, `permission_type`, `parent_id`, `sort_no`)
SELECT 'settings:project-settings', '项目设置', 'menu',
       (SELECT `id` FROM `permission` WHERE `permission_code` = 'settings'),
       405
WHERE NOT EXISTS (
  SELECT 1 FROM `permission` WHERE `permission_code` = 'settings:project-settings'
);

INSERT INTO `role_permission` (`role_id`, `permission_id`)
SELECT r.`id`, p.`id`
FROM `role` r
JOIN `permission` p ON p.`permission_code` = 'settings:project-settings'
WHERE r.`role_code` = 'super_admin'
  AND NOT EXISTS (
    SELECT 1 FROM `role_permission` rp
    WHERE rp.`role_id` = r.`id` AND rp.`permission_id` = p.`id`
  );
