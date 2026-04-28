SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'task' AND COLUMN_NAME = 'task_type'
  ),
  'SELECT 1',
  'ALTER TABLE `task` ADD COLUMN `task_type` VARCHAR(30) NULL COMMENT ''任务类型: three_special/four_special'' AFTER `service_type`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'task' AND INDEX_NAME = 'idx_task_task_type'
  ),
  'SELECT 1',
  'ALTER TABLE `task` ADD KEY `idx_task_task_type` (`task_type`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

UPDATE `task` t
LEFT JOIN (
  SELECT `task_id`, COUNT(DISTINCT `service_type`) AS service_type_count
  FROM `task_service_item`
  GROUP BY `task_id`
) tsi ON tsi.task_id = t.id
SET t.task_type = CASE
  WHEN COALESCE(tsi.service_type_count, IF(t.service_type IS NULL OR t.service_type = '', 0, 1)) = 3 THEN 'three_special'
  WHEN COALESCE(tsi.service_type_count, IF(t.service_type IS NULL OR t.service_type = '', 0, 1)) = 4 THEN 'four_special'
  ELSE t.task_type
END
WHERE t.task_type IS NULL OR t.task_type = '';
