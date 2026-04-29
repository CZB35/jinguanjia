-- 支持师傅端按任务服务环节拆分执行记录

SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'task_execution' AND COLUMN_NAME = 'service_item_id'
  ),
  'SELECT 1',
  'ALTER TABLE `task_execution` ADD COLUMN `service_item_id` BIGINT NULL COMMENT ''任务服务环节ID'' AFTER `task_id`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'task_execution' AND COLUMN_NAME = 'service_type'
  ),
  'SELECT 1',
  'ALTER TABLE `task_execution` ADD COLUMN `service_type` VARCHAR(30) NULL COMMENT ''服务环节: measure/install/ironing/cleaning'' AFTER `service_item_id`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'task_execution' AND INDEX_NAME = 'idx_task_execution_service_item_id'
  ),
  'SELECT 1',
  'ALTER TABLE `task_execution` ADD KEY `idx_task_execution_service_item_id` (`service_item_id`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'task_execution' AND INDEX_NAME = 'idx_task_execution_task_service'
  ),
  'SELECT 1',
  'ALTER TABLE `task_execution` ADD KEY `idx_task_execution_task_service` (`task_id`, `service_type`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

UPDATE `task_execution` te
JOIN `task` t ON t.`id` = te.`task_id`
LEFT JOIN `task_service_item` tsi ON tsi.`task_id` = te.`task_id` AND tsi.`service_type` = t.`service_type`
SET te.`service_item_id` = tsi.`id`,
    te.`service_type` = t.`service_type`
WHERE te.`service_item_id` IS NULL
  AND te.`service_type` IS NULL;
