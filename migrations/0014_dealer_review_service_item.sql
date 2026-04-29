-- 支持经销商按任务服务环节分别评价

SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'dealer_review' AND COLUMN_NAME = 'service_item_id'
  ),
  'SELECT 1',
  'ALTER TABLE `dealer_review` ADD COLUMN `service_item_id` BIGINT NULL COMMENT ''任务服务环节ID'' AFTER `task_id`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'dealer_review' AND COLUMN_NAME = 'service_type'
  ),
  'SELECT 1',
  'ALTER TABLE `dealer_review` ADD COLUMN `service_type` VARCHAR(30) NULL COMMENT ''服务环节: measure/install/ironing/cleaning'' AFTER `service_item_id`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'dealer_review' AND INDEX_NAME = 'uk_dealer_review_task_id'
  ),
  'ALTER TABLE `dealer_review` DROP INDEX `uk_dealer_review_task_id`',
  'SELECT 1'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'dealer_review' AND INDEX_NAME = 'idx_dealer_review_service_item_id'
  ),
  'SELECT 1',
  'ALTER TABLE `dealer_review` ADD KEY `idx_dealer_review_service_item_id` (`service_item_id`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

UPDATE `dealer_review` dr
JOIN `task` t ON t.`id` = dr.`task_id`
LEFT JOIN `task_service_item` tsi ON tsi.`task_id` = dr.`task_id`
  AND tsi.`service_type` = COALESCE(dr.`service_type`, t.`service_type`)
SET dr.`service_item_id` = COALESCE(dr.`service_item_id`, tsi.`id`),
    dr.`service_type` = COALESCE(dr.`service_type`, t.`service_type`, tsi.`service_type`)
WHERE dr.`service_item_id` IS NULL
   OR dr.`service_type` IS NULL;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'dealer_review' AND INDEX_NAME = 'uk_dealer_review_task_service'
  ),
  'SELECT 1',
  'ALTER TABLE `dealer_review` ADD UNIQUE KEY `uk_dealer_review_task_service` (`task_id`, `service_type`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;
