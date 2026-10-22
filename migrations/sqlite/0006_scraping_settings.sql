CREATE TABLE scraping_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    config TEXT NOT NULL
);
