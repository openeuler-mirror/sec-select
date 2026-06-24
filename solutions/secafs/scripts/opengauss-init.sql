-- OpenGauss 初始化脚本（容器首次启动时，以初始管理员身份运行）
--
-- 目标：让 `secafs init opengauss://secafs:...@host/<任意库>` 开箱即用，
-- 避免 OpenGauss/PG15 下普通用户对 public schema 报 "permission denied"。

-- 将 secafs 设为 SYSADMIN：可在任意数据库（包括之后新建的库）的 public
-- schema 中建表/建索引/建触发器，无需为每个新库单独 GRANT ON SCHEMA public。
-- 这是新建库授权摩擦的根治：CREATE DATABASE 后直接可用。
ALTER ROLE secafs SYSADMIN;

-- 允许 secafs 创建新数据库（platform 多租户场景 / 每个 agent 一个库）。
ALTER ROLE secafs CREATEDB;

-- 创建默认开发库，匹配 scripts/dev-up.sh 输出的 opengauss://.../secafs URL。
-- （首次初始化时执行一次；已存在则忽略本行即可。）
CREATE DATABASE secafs OWNER secafs;
