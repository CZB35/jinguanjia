-- 为同批多服务类型真实多单增加分组号，便于一次分配整组任务

SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'task' AND COLUMN_NAME = 'task_group_no'
  ),
  'SELECT 1',
  'ALTER TABLE `task` ADD COLUMN `task_group_no` VARCHAR(64) NULL COMMENT ''同批任务分组号'' AFTER `service_type`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'task' AND INDEX_NAME = 'idx_task_group_no'
  ),
  'SELECT 1',
  'ALTER TABLE `task` ADD KEY `idx_task_group_no` (`task_group_no`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

UPDATE `task`
SET `task_group_no` = CONCAT('legacy-', `id`)
WHERE `task_group_no` IS NULL;
