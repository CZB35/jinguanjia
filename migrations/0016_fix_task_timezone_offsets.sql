-- 修复历史任务环节时间按 UTC 写入导致展示慢 8 小时的问题

SET NAMES utf8mb4;

UPDATE `task_assignment`
SET `assigned_at` = DATE_ADD(`assigned_at`, INTERVAL 8 HOUR)
WHERE `assigned_at` IS NOT NULL;

UPDATE `task_execution`
SET `started_at` = DATE_ADD(`started_at`, INTERVAL 8 HOUR),
    `submitted_at` = CASE
      WHEN `submitted_at` IS NULL THEN NULL
      ELSE DATE_ADD(`submitted_at`, INTERVAL 8 HOUR)
    END
WHERE `started_at` IS NOT NULL
   OR `submitted_at` IS NOT NULL;

UPDATE `task_service_item`
SET `started_at` = CASE
      WHEN `started_at` IS NULL THEN NULL
      ELSE DATE_ADD(`started_at`, INTERVAL 8 HOUR)
    END,
    `completed_at` = CASE
      WHEN `completed_at` IS NULL THEN NULL
      ELSE DATE_ADD(`completed_at`, INTERVAL 8 HOUR)
    END
WHERE `started_at` IS NOT NULL
   OR `completed_at` IS NOT NULL;

UPDATE `evidence`
SET `shot_at` = DATE_ADD(`shot_at`, INTERVAL 8 HOUR)
WHERE `shot_at` IS NOT NULL;
