# Read-only аудит `database/main.sqlite3`

Дата проверки: 2026-09-19. Проверка выполнена без изменения файла БД. Все диагностические SQL-запросы запускались через `sqlite3 -readonly`; `ANALYZE`, `VACUUM`, `PRAGMA optimize` и другие записывающие операции к исходной базе не применялись.

## Резюме

База технически исправна и для нынешних 69 строк работает быстро. `PRAGMA integrity_check` и `quick_check` возвращают `ok`, свободных страниц нет, уникальность `app_path` соблюдена, типы и даты текущих данных корректны. Точечные операции по `app_path` используют существующий уникальный индекс.

Основные проблемы находятся не в размере файла, а в модели данных и коде доступа:

1. Приложение не использует проверенный файл репозитория: `database.init.get_db_path()` при текущем окружении возвращает `/home/sakura/.local/share/luma/main.sqlite3`. Это отдельный файл с другим SHA-256 и другими временами записей. Есть риск проверять, мигрировать или поставлять не ту БД.
2. `INSERT OR IGNORE` навсегда оставляет устаревшие `name`, `categories`, `icon_path`, `command` и `last_checked` у уже известного `app_path`.
3. Первичное сканирование записывает одно и то же текущее время одновременно в `last_used` и `last_checked`. Поэтому все 69 приложений выглядят «недавно использованными», хотя это время обнаружения, а не запуска.
4. Категории хранятся как строка с разделителем. Запрос по категории несаргируемый и делает полный проход. Метод `filter_apps_by_categories()` при текущих 46 категориях потенциально открывает 47 соединений и выполняет 46 полных проходов таблицы.
5. Каждая операция сервиса открывает новое соединение. На текущем объёме это терпимо, но создаёт лишние накладные расходы и усложняет единообразную настройку `foreign_keys`, таймаутов и режима журнала.

## Объект аудита и расхождение путей

Проверенный файл:

```text
/run/media/sakura/1a177757-dd80-4b2f-8b81-d3db963ca160/projects/luma/src/database/main.sqlite3
SHA-256: 22c78971d20d73873a84c179add7a49e0dd5882bdf4e32d4bfbbcbf952227332
```

Код `database/init.py` вычисляет путь через `$XDG_DATA_HOME/luma/main.sqlite3`, а при отсутствии переменной — через `~/.local/share/luma/main.sqlite3`. В текущем окружении `$XDG_DATA_HOME` не задан, поэтому runtime-файл находится здесь:

```text
/home/sakura/.local/share/luma/main.sqlite3
SHA-256: 5b4a1b19ce29a6617451a5fb9ace8777c458b519b03134abca8fd31319ca6b46
```

Файлы не идентичны (`cmp` вернул различие). Оба на момент проверки содержат 69 строк, но диапазон `last_used` в проверенной БД — `17:26:42–17:26:43`, а в runtime-БД — `20:47:53–20:48:07`. Детальный аудит ниже относится только к `database/main.sqlite3`.

## Схема

Пользовательская таблица одна:

```sql
CREATE TABLE applications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    categories TEXT,
    app_path TEXT NOT NULL UNIQUE,
    icon_path TEXT,
    command TEXT,
    last_used TEXT DEFAULT CURRENT_TIMESTAMP,
    last_checked TEXT
);
```

Объекты:

| Объект | Назначение |
|---|---|
| `applications` | 69 приложений |
| `sqlite_autoindex_applications_1` | уникальный B-tree по `app_path` |
| `sqlite_sequence` | счётчик, созданный из-за `AUTOINCREMENT`; значение 69 |

Пользовательских индексов, представлений и триггеров нет. Внешних ключей нет. `user_version = 0`, то есть версионирование схемы не ведётся.

Замечания по схеме:

- `id` в прочитанном коде не используется: `_application_from_row()` начинает с `row[1]`.
- `AUTOINCREMENT` здесь не даёт полезной гарантии приложению, но создаёт `sqlite_sequence` и запрещает повторное использование старых ROWID. Обычного `INTEGER PRIMARY KEY` достаточно, если ID нужно сохранить.
- `categories` нарушает первую нормальную форму: в одном поле лежит набор значений.
- Времена хранятся как naive-текст без часового пояса. При этом `CURRENT_TIMESTAMP` в SQLite — UTC, а Python-код записывает `datetime.now()` в локальном времени. Использование default и явной записи способно смешать две временные шкалы.
- Для `name`, `app_path`, дат и категорий отсутствуют `CHECK`-ограничения. Текущие строки корректны, но схема не предотвращает пустые значения или неверный формат в будущем.
- Код зависит от порядка колонок через `SELECT *` и позиционные индексы. Добавление/перестановка столбца легко сломает преобразование в модель.

## Размер и физическое размещение

| Метрика | Значение |
|---|---:|
| Размер файла | 32 768 байт |
| Размер страницы | 4 096 байт |
| Всего страниц | 8 |
| Свободных страниц (`freelist_count`) | 0 |
| `auto_vacuum` | 0 (`NONE`) |
| Кодировка | UTF-8 |
| Журнал | `DELETE` |

Распределение по `dbstat`:

| Объект | Страниц | Выделено | Payload | Заполнение payload |
|---|---:|---:|---:|---:|
| `applications` | 5 | 20 480 B | 13 083 B | 63,9% |
| уникальный индекс `app_path` | 1 | 4 096 B | 3 591 B | 87,7% |
| `sqlite_schema` | 1 | 4 096 B | 501 B | 12,2% |
| `sqlite_sequence` | 1 | 4 096 B | 16 B | 0,4% |

`VACUUM` сейчас не нужен: freelist пуст, а файл уже занимает минимальный практически значимый объём для своей схемы. Удаление `AUTOINCREMENT` при будущей миграции позволит отказаться от отдельной страницы `sqlite_sequence`, но экономия 4 KiB сама по себе не является причиной миграции.

## Данные и целостность

| Проверка | Результат |
|---|---:|
| Строк | 69 |
| Диапазон ID | 1–69, пропусков нет |
| Уникальных `app_path` | 69 |
| Уникальных `name` | 66 |
| NULL в `name`, `app_path`, `command`, `last_used`, `last_checked` | 0 |
| NULL в `categories` | 0 |
| Пустых `categories` | 17 |
| NULL в `icon_path` | 23 |
| Некорректных типов хранения | 0 |
| Невалидных дат по `strftime` | 0 |
| Несуществующих путей `app_path` | 0 |
| Дубликатов категории внутри одного приложения | 0 |
| Коллизий путей без учёта регистра | 0 |

Три повторяющихся отображаемых имени (`Code - OSS - URL Handler`, `Portal`, `Sunshine`) относятся к разным `.desktop`-файлам; это не нарушение, потому что идентичностью служит `app_path`.

Категории:

- 154 назначения категорий;
- 46 уникальных категорий, в том числе после приведения регистра;
- 41 уникальная строковая комбинация категорий;
- самые частые: `Utility` — 15, `GTK` — 14, `GNOME` — 13, `System` — 10;
- 3 непустые строки не заканчиваются `;`, хотя большинство заканчивается; текущий запрос это переносит, но формат не канонический.

Все 69 строк имеют `last_checked = last_used`. 49 строк получили `2026-09-17 17:26:42`, остальные 20 — `17:26:43`. Это сильное свидетельство того, что `last_used` заполнен временем сканирования, а не реального запуска.

## PRAGMA и статистика планировщика

Наблюдавшиеся значения: SQLite 3.53.4, `journal_mode=delete`, `synchronous=2`, `foreign_keys=0`, `cache_size=-2000`, `mmap_size=0`, `locking_mode=normal`. Таблицы `sqlite_stat1` нет, следовательно `ANALYZE` на этом файле не запускался.

Важно: часть PRAGMA относится к конкретному соединению. Значения CLI-соединения не следует автоматически считать настройками `aiosqlite`. Однако код приложения явно не выполняет `PRAGMA foreign_keys=ON` и не задаёт единый профиль соединения; после добавления внешних ключей это станет ошибкой целостности. `journal_mode=DELETE` хранится на уровне файла и соответствует проверенному файлу.

Отсутствие статистики сейчас почти ничего не стоит: таблица микроскопическая, схема простая, а селективный запрос имеет однозначный уникальный индекс. После миграции/существенного роста статистику стоит создать.

## Планы запросов и фактические пути кода

Результат `EXPLAIN QUERY PLAN`:

| Операция | План | Оценка |
|---|---|---|
| `SELECT * FROM applications` | `SCAN applications` | ожидаемо, нужны все строки |
| `SELECT categories FROM applications` | `SCAN applications` | ожидаемо, но лишний отдельный проход |
| `WHERE app_path = ?` | `SEARCH ... USING INDEX sqlite_autoindex...` | оптимально |
| `UPDATE ... WHERE app_path = ?` | тот же уникальный индекс | оптимально |
| `DELETE ... WHERE app_path = ?` | тот же уникальный индекс | оптимально |
| поиск категории через `';' || categories || ';' LIKE ...` | `SCAN applications` | индекс применить невозможно |
| `ORDER BY last_used DESC` | scan + `USE TEMP B-TREE FOR ORDER BY` | индекса нет; сейчас сортировка выполняется в Python |

`ApplicationData.refresh()` читает все строки, а затем UI неоднократно сортирует/фильтрует уже загруженный список. Для 69 записей это рациональнее сложных индексов.

Метод `filter_apps_by_categories()` сейчас нигде больше не вызывается, но при вызове:

1. делает полный запрос всех строк `categories`;
2. получает 46 категорий;
3. для каждой категории отдельно вызывает `asyncio.run(get_apps_by_category(...))`;
4. каждый вызов открывает новое соединение и выполняет полный scan.

Итого для текущих данных: 47 SQL-запросов/соединений и 46 повторных полных проходов. Это N+1-паттерн с квадратичным ростом относительно числа категорий и приложений.

Поиск по имени выполняется через `rapidfuzz` в Python. Обычный B-tree индекс по `name` не ускорит fuzzy/substring-сопоставление, поэтому сейчас добавлять его бессмысленно.

## Приоритизированные рекомендации

### P0 — корректность и однозначность данных

#### 1. Сделать путь к БД явным

Нужно решить, чем является `database/main.sqlite3`: тестовым fixture/seed-файлом или рабочей БД. Рабочую изменяемую БД разумно хранить только в XDG data directory, а репозиторный файл либо убрать из поставки, либо назвать `seed.sqlite3`/`test.sqlite3` и использовать явно в тестах.

Рекомендуется принимать путь через один объект конфигурации, логировать его при старте и в тесте проверять:

```python
db_path = get_db_path().resolve()
logger.info("Using SQLite database: %s", db_path)
```

Не следует запускать миграции «по относительному пути», если runtime-код использует другой файл.

#### 2. Заменить `INSERT OR IGNORE` на UPSERT

Сканирование должно обновлять изменяемые метаданные и `last_checked`, но сохранять реальное время последнего запуска:

```sql
INSERT INTO applications (
    name, categories, app_path, icon_path, command, last_checked
) VALUES (?, ?, ?, ?, ?, ?)
ON CONFLICT(app_path) DO UPDATE SET
    name         = excluded.name,
    categories   = excluded.categories,
    icon_path    = excluded.icon_path,
    command      = excluded.command,
    last_checked = excluded.last_checked;
```

Для новой записи `last_used` должен быть `NULL`; обновлять его следует только после успешного запуска приложения. При этом схему лучше изменить с `last_used TEXT DEFAULT CURRENT_TIMESTAMP` на nullable-поле без default.

#### 3. Выбрать одну временную шкалу

Предпочтительный простой вариант — хранить Unix time в UTC как `INTEGER`:

```sql
last_used    INTEGER,
last_checked INTEGER NOT NULL
```

Альтернатива — строгий ISO-8601 UTC (`2026-09-19T10:20:30Z`). Нельзя смешивать локальный `datetime.now()` с SQLite `CURRENT_TIMESTAMP`.

### P1 — устранение лишних проходов и соединений

#### 4. Для текущего масштаба группировать уже загруженные приложения

Самое дешёвое решение — удалить DB-цикл из `filter_apps_by_categories()` и использовать `apps_obj._apps`, как уже делает `frontend.buttons.configure_badges()`. Это превращает 47 подключений и 46 scan в один проход Python по 69 объектам.

#### 5. При росте данных нормализовать категории

Если категории должны фильтроваться SQL-запросами или число приложений вырастет, создать таблицу связи:

```sql
PRAGMA foreign_keys = ON;

CREATE TABLE application_categories (
    application_id INTEGER NOT NULL
        REFERENCES applications(id) ON DELETE CASCADE,
    category TEXT NOT NULL CHECK (length(trim(category)) > 0),
    PRIMARY KEY (application_id, category)
) WITHOUT ROWID;

CREATE INDEX application_categories_by_category
    ON application_categories(category, application_id);
```

Тогда запрос становится индексируемым:

```sql
SELECT a.id, a.name, a.app_path, a.icon_path, a.command,
       a.last_used, a.last_checked
FROM application_categories AS ac
JOIN applications AS a ON a.id = ac.application_id
WHERE ac.category = ?;
```

Миграцию разделённых `;` значений безопаснее выполнить кодом приложения в одной транзакции: trim, отброс пустых значений, `INSERT OR IGNORE` в таблицу связи, затем верификация числа связей (ожидается 154 для текущего файла).

#### 6. Управлять соединением централизованно

Вместо нового соединения на каждый метод использовать один lifecycle-managed connection либо небольшой repository/context manager. На каждом соединении явно задавать профиль:

```sql
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

Если UI и watcher действительно пишут параллельно, после нагрузочного теста можно включить один раз:

```sql
PRAGMA journal_mode = WAL;
```

WAL улучшит сосуществование чтения и записи, но для 69 строк не является обязательным. Нужно корректно закрывать соединение и учитывать `-wal`/`-shm` при резервном копировании. Для нескольких изменений использовать одну явную транзакцию, как уже фактически делает `executemany()`.

### P2 — сопровождаемость и точечные улучшения

#### 7. Ввести миграции через `user_version`

После создания резервной копии и проверки правильного runtime-пути миграции выполнять транзакционно:

```sql
BEGIN IMMEDIATE;
-- CREATE new tables / copy and validate rows / rename
PRAGMA user_version = 1;
COMMIT;
```

До `DROP TABLE` необходимо сверить количество строк, уникальность `app_path`, количество связей категорий и выполнить `PRAGMA foreign_key_check`. При ошибке — `ROLLBACK`.

#### 8. Убрать ненужный `AUTOINCREMENT`

При следующей миграции заменить `id INTEGER PRIMARY KEY AUTOINCREMENT` на `id INTEGER PRIMARY KEY`. Если ни один внешний/API-контракт не использует ID, возможен и `app_path TEXT PRIMARY KEY` в `WITHOUT ROWID`, но для будущих связей категорий компактный integer ID удобнее.

#### 9. Использовать явные колонки и row factory

Вместо `SELECT *`:

```sql
SELECT name, categories, app_path, icon_path, command,
       last_used, last_checked
FROM applications;
```

Это устраняет хрупкую зависимость `_application_from_row()` от позиции `id` и упрощает миграции. Ещё надёжнее использовать `aiosqlite.Row` и обращения по именам.

#### 10. Добавлять индекс времени только под реальный SQL-запрос

Если сортировка будет перенесена в SQLite и таблица станет заметно больше:

```sql
CREATE INDEX applications_last_used_desc
    ON applications(last_used DESC);

SELECT name, categories, app_path, icon_path, command,
       last_used, last_checked
FROM applications
ORDER BY last_used DESC;
```

При 69 строках и сортировке в памяти этот индекс лишь увеличит стоимость записи и размер файла, поэтому сейчас его создавать не следует.

#### 11. Обслуживание после миграции, не до неё

После изменения схемы и массовой загрузки:

```sql
ANALYZE;
PRAGMA optimize;
PRAGMA integrity_check;
PRAGMA foreign_key_check;
```

`VACUUM` запускать только при реальном большом freelist/сжатии файла и с достаточным свободным диском. Сейчас он не нужен.

## Production-ready модель категорий

### Выбор модели

Рекомендуемая целевая модель — обычная связь many-to-many:

- `applications` остаётся владельцем приложения;
- `categories` содержит одну каноническую категорию;
- `application_categories` содержит только связи.

Это лучше строки с `;` и JSON, потому что обеспечивает ссылочную целостность, исключает повтор категории у приложения и индексирует поиск в обоих направлениях. Для 69 приложений разница во времени незаметна, но схема устраняет N+1 и остаётся пригодной при росте без смены формата.

Краткое сравнение:

| Вариант | Плюсы | Минусы | Решение |
|---|---|---|---|
| Строка с `;` | простая запись, совпадает с форматом `.desktop` | нет FK/UNIQUE по элементам, полный scan, ручный escaping/parsing | удалить после миграции |
| JSON-массив | корректнее сериализует список, есть `json_each` | всё ещё нет FK и простой уникальности, поиск требует раскрытия массива/выражений | уместен только как сырой документ, не как рабочая модель |
| Справочник + junction table | целостность, дедупликация, эффективные ANY/ALL/count, обычные JOIN | две таблицы и транзакционное обновление связей | рекомендуемый вариант |

### Целевой DDL

SQLite runtime проекта — 3.53.4, поэтому можно использовать `STRICT`. Ограничение длины в 128 символов защищает от ошибочного импорта и с большим запасом покрывает существующие значения.

```sql
PRAGMA foreign_keys = ON; -- выполнять на каждом соединении

CREATE TABLE categories (
    id            INTEGER PRIMARY KEY,
    category_key  TEXT NOT NULL
        UNIQUE
        CHECK (
            category_key = trim(category_key)
            AND length(category_key) BETWEEN 1 AND 128
        ),
    display_name  TEXT NOT NULL
        CHECK (
            display_name = trim(display_name)
            AND length(display_name) BETWEEN 1 AND 128
        )
) STRICT;

CREATE TABLE application_categories (
    application_id INTEGER NOT NULL
        REFERENCES applications(id)
        ON UPDATE RESTRICT
        ON DELETE CASCADE,
    category_id INTEGER NOT NULL
        REFERENCES categories(id)
        ON UPDATE RESTRICT
        ON DELETE CASCADE,
    PRIMARY KEY (application_id, category_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX application_categories_by_category
    ON application_categories(category_id, application_id);
```

Этого набора индексов достаточно:

- `UNIQUE(categories.category_key)` создаёт индекс для разрешения входного ключа в `category_id`;
- PK junction table покрывает «категории приложения»;
- один обратный индекс покрывает «приложения категории», ANY/ALL и count.

Отдельные индексы только на `application_id`, `category_id` или `display_name` дублировали бы перечисленные префиксы и не нужны. `ON DELETE CASCADE` у обеих ссылок делает удаление приложения или категории безопасным. `ON UPDATE RESTRICT` выбран намеренно: surrogate ID являются стабильной идентичностью и не должны перенумеровываться. Переименование категории меняет `display_name`/`category_key`, но не ID.

После удаления приложения категория может остаться с count 0. Это полезно, если справочник управляется отдельно. Если категории являются только производными от сканирования, неиспользуемые строки следует удалять одной cleanup-операцией после полного успешного сканирования, но не после обработки каждого приложения:

```sql
DELETE FROM categories AS c
WHERE NOT EXISTS (
    SELECT 1
    FROM application_categories AS ac
    WHERE ac.category_id = c.id
);
```

### Канонизация, регистр и порядок

До SQL приложение должно для каждого токена выполнить:

1. разделение исходного `.desktop` значения по `;`;
2. удаление пустых токенов и внешних пробелов;
3. Unicode NFKC normalization;
4. `casefold()` для получения `category_key`;
5. дедупликацию ключей внутри одного приложения.

SQLite `lower()` обрабатывает Unicode не так полно, как Python `casefold()`, поэтому production-код не должен полагаться на `lower()` для новых значений. В текущем снимке все 154 токена ASCII, поэтому `lower(trim(token))` безопасен именно для одноразовой миграции. Например, `Qt` хранится как `display_name='Qt'`, `category_key='qt'`; варианты `QT`/`qt` второй категории не создают.

`display_name` следует брать из фиксированной карты стандартных Freedesktop-категорий, а для неизвестных значений — сохранять первое встреченное написание. Обычный повторный scan не должен менять его на написание последнего случайно обработанного файла; переименование делается отдельной явной операцией.

Категории являются множеством, а не упорядоченным списком, поэтому исходный порядок хранить не рекомендуется. Для стабильного API использовать `ORDER BY category_key`, для локализованного UI — сортировать `display_name` в приложении с нужной locale. Если появится подтверждённое требование воспроизводить исходный порядок, можно добавить `ordinal INTEGER NOT NULL CHECK(ordinal >= 0)` и `UNIQUE(application_id, ordinal)`, но это создаст ещё один индекс и усложнит UPSERT; сейчас это лишнее.

Исходную колонку `applications.categories` в целевой схеме следует удалить: две копии одного факта неизбежно расходятся. В переходной миграции её можно оставить только на один совместимый релиз, но должен быть один владелец записи и явная дата удаления. Для диагностики исходный `.desktop` файл уже доступен по `app_path`; отдельное `raw_categories` не требуется.

### Миграция текущих 154 связей

Перед миграцией необходимо остановить scanner/watcher, сделать backup **фактического runtime-файла**, а не файла с похожим именем, и поставить новый код, который больше не читает позиционное `row[2]`. Ниже миграция для проверенного снимка. Она динамически вычисляет ожидаемые значения, проверяет результат через `CHECK`, а дополнительно ожидается 46 категорий и 154 уникальные связи.

```sql
PRAGMA foreign_keys = ON;
BEGIN IMMEDIATE;

CREATE TEMP TABLE _category_migration_expected AS
WITH RECURSIVE split(application_id, rest, token) AS (
    SELECT id, coalesce(categories, '') || ';', ''
    FROM applications
    UNION ALL
    SELECT application_id,
           substr(rest, instr(rest, ';') + 1),
           trim(substr(rest, 1, instr(rest, ';') - 1))
    FROM split
    WHERE instr(rest, ';') > 0
), tokens AS (
    SELECT application_id,
           token,
           lower(token) AS category_key
    FROM split
    WHERE token <> ''
)
SELECT
    (SELECT count(DISTINCT category_key) FROM tokens) AS category_count,
    (SELECT count(*) FROM (
        SELECT DISTINCT application_id, category_key FROM tokens
    )) AS link_count;

CREATE TABLE categories (
    id            INTEGER PRIMARY KEY,
    category_key  TEXT NOT NULL UNIQUE
        CHECK (category_key = trim(category_key)
               AND length(category_key) BETWEEN 1 AND 128),
    display_name  TEXT NOT NULL
        CHECK (display_name = trim(display_name)
               AND length(display_name) BETWEEN 1 AND 128)
) STRICT;

CREATE TABLE application_categories (
    application_id INTEGER NOT NULL
        REFERENCES applications(id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    category_id INTEGER NOT NULL
        REFERENCES categories(id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    PRIMARY KEY (application_id, category_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX application_categories_by_category
    ON application_categories(category_id, application_id);

WITH RECURSIVE split(rest, token) AS (
    SELECT coalesce(categories, '') || ';', '' FROM applications
    UNION ALL
    SELECT substr(rest, instr(rest, ';') + 1),
           trim(substr(rest, 1, instr(rest, ';') - 1))
    FROM split
    WHERE instr(rest, ';') > 0
), tokens AS (
    SELECT token, lower(token) AS category_key
    FROM split
    WHERE token <> ''
)
INSERT INTO categories(category_key, display_name)
SELECT category_key, min(token)
FROM tokens
GROUP BY category_key;

WITH RECURSIVE split(application_id, rest, token) AS (
    SELECT id, coalesce(categories, '') || ';', '' FROM applications
    UNION ALL
    SELECT application_id,
           substr(rest, instr(rest, ';') + 1),
           trim(substr(rest, 1, instr(rest, ';') - 1))
    FROM split
    WHERE instr(rest, ';') > 0
), tokens AS (
    SELECT DISTINCT application_id, lower(token) AS category_key
    FROM split
    WHERE token <> ''
)
INSERT INTO application_categories(application_id, category_id)
SELECT t.application_id, c.id
FROM tokens AS t
JOIN categories AS c USING (category_key);

-- Любое несовпадение прерывает INSERT ограничением CHECK.
CREATE TEMP TABLE _category_migration_assert (
    ok INTEGER NOT NULL CHECK (ok = 1)
);

INSERT INTO _category_migration_assert(ok)
SELECT (e.category_count = (SELECT count(*) FROM categories)
        AND e.link_count = (SELECT count(*) FROM application_categories)
        AND e.category_count = 46
        AND e.link_count = 154)
FROM _category_migration_expected AS e;

INSERT INTO _category_migration_assert(ok)
SELECT NOT EXISTS (SELECT 1 FROM pragma_foreign_key_check);

-- Выполнять только одновременно с переходом кода на новую схему.
ALTER TABLE applications DROP COLUMN categories;

PRAGMA user_version = 2;
DROP TABLE _category_migration_assert;
DROP TABLE _category_migration_expected;
COMMIT;

PRAGMA integrity_check;
PRAGMA foreign_key_check;
ANALYZE;
PRAGMA optimize;
```

Жёсткие `46/154` относятся к аудируемому снимку. Если к моменту релиза scanner легитимно изменит данные, сначала заново снять ожидаемые числа и обновить guard; динамическое сравнение всё равно должно остаться. При любой ошибке до `COMMIT` выполнить `ROLLBACK`.

### Рабочие запросы

Категории одного приложения, в стабильном каноническом порядке:

```sql
SELECT c.id, c.category_key, c.display_name
FROM application_categories AS ac
JOIN categories AS c ON c.id = ac.category_id
WHERE ac.application_id = ?
ORDER BY c.category_key;
```

Приложения одной категории (`?` — уже канонизированный key):

```sql
SELECT a.id, a.name, a.app_path, a.icon_path, a.command,
       a.last_used, a.last_checked
FROM categories AS c
JOIN application_categories AS ac ON ac.category_id = c.id
JOIN applications AS a ON a.id = ac.application_id
WHERE c.category_key = ?
ORDER BY a.name, a.id;
```

Приложения, имеющие **любую** из переданных категорий (ANY):

```sql
WITH requested(category_key) AS (
    VALUES (?), (?), (?)
), wanted AS (
    SELECT DISTINCT category_key FROM requested
), matched AS (
    SELECT DISTINCT ac.application_id
    FROM wanted AS w
    JOIN categories AS c USING (category_key)
    JOIN application_categories AS ac ON ac.category_id = c.id
)
SELECT a.id, a.name, a.app_path, a.icon_path, a.command,
       a.last_used, a.last_checked
FROM matched AS m
JOIN applications AS a ON a.id = m.application_id
ORDER BY a.name, a.id;
```

Приложения, имеющие **все** переданные категории (ALL):

```sql
WITH requested(category_key) AS (
    VALUES (?), (?), (?)
), wanted AS (
    SELECT DISTINCT category_key FROM requested
), matched AS (
    SELECT ac.application_id
    FROM wanted AS w
    JOIN categories AS c USING (category_key)
    JOIN application_categories AS ac ON ac.category_id = c.id
    GROUP BY ac.application_id
    HAVING count(*) = (SELECT count(*) FROM wanted)
)
SELECT a.id, a.name, a.app_path, a.icon_path, a.command,
       a.last_used, a.last_checked
FROM matched AS m
JOIN applications AS a ON a.id = m.application_id
ORDER BY a.name, a.id;
```

Для пустого набора следует определить контракт в приложении: обычно ANY возвращает 0 строк, а ALL — все приложения. Приведённый ALL-запрос с пустым `VALUES` потребует отдельной ветки, иначе matched также будет пуст.

Справочник категорий с числом приложений, включая неиспользуемые:

```sql
SELECT c.id, c.category_key, c.display_name,
       count(ac.application_id) AS application_count
FROM categories AS c
LEFT JOIN application_categories AS ac ON ac.category_id = c.id
GROUP BY c.id
ORDER BY c.category_key;
```

### Атомарный UPSERT при повторном сканировании

Одна обработка приложения должна быть одной транзакцией:

```sql
BEGIN IMMEDIATE;

INSERT INTO applications (
    name, app_path, icon_path, command, last_checked
) VALUES (?, ?, ?, ?, ?)
ON CONFLICT(app_path) DO UPDATE SET
    name         = excluded.name,
    icon_path    = excluded.icon_path,
    command      = excluded.command,
    last_checked = excluded.last_checked
RETURNING id;

-- Для каждого уникального (category_key, display_name):
INSERT INTO categories(category_key, display_name)
VALUES (?, ?)
ON CONFLICT(category_key) DO NOTHING;

-- Получить ID категорий пакетно либо этим запросом для каждого key:
SELECT id, category_key
FROM categories
WHERE category_key IN (?, ?, ?);

-- application_id — ID из RETURNING. Полная замена связей безопасна,
-- потому что она находится в той же транзакции.
DELETE FROM application_categories WHERE application_id = ?;

INSERT INTO application_categories(application_id, category_id)
VALUES (?, ?), (?, ?), (?, ?);

COMMIT;
```

`last_used` намеренно отсутствует в UPDATE: повторный scan не должен менять историю запуска. `ON CONFLICT ... DO NOTHING` для категории сохраняет выбранное ранее `display_name` и не создаёт бессмысленную UPDATE-запись. Перед `INSERT` список category ID должен быть дедуплицирован; PK остаётся последней защитой.

Для 69 приложений «DELETE всех старых связей + INSERT текущих» проще и достаточно быстро. При тысячах часто пересканируемых приложений можно вычислять diff и удалять/добавлять только изменения, не меняя схему. Удаление исчезнувших приложений нужно выполнять после полного успешного обхода набора источников; каскад автоматически удалит их связи. Очистку orphan-категорий также запускать только после завершённого полного scan.

## Предлагаемый порядок внедрения

1. Зафиксировать и протестировать единственный runtime-путь к БД.
2. Исправить семантику `last_used`, перейти на UPSERT и обновление `last_checked`.
3. Убрать N+1 по категориям, сначала простым группированием уже загруженных объектов.
4. Добавить миграционный механизм и `user_version`.
5. Нормализовать категории только если нужен SQL-фильтр/масштабирование.
6. Централизовать соединение и PRAGMA; WAL включать только при подтверждённой конкуренции.
7. После миграции выполнить `ANALYZE`, проверки целостности и повторно снять `EXPLAIN QUERY PLAN`.

## Итоговая оценка

Для текущих 69 строк база здорова, компактна и не требует ни `VACUUM`, ни дополнительных индексов. Наибольший практический выигрыш дадут не низкоуровневые настройки SQLite, а устранение расхождения путей, корректный UPSERT, разделение `last_used`/`last_checked` и удаление потенциального N+1-паттерна категорий. Нормализация и WAL — подготовка к росту и конкурентному доступу, а не срочная оптимизация текущего файла.
