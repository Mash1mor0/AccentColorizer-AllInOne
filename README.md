# AccentColorizer (Rust) - All-In-One (In-Memory Hook & Theme Colorizer)

Высокопроизводительная утилита на Rust для полной колоризации интерфейса Windows 10 и 11 (21H2 – 26H2) **полностью в оперативной памяти без изменения системных файлов на диске**.

---

## 🌟 Ключевые возможности

1. **Работа исключительно в оперативной памяти (Zero Disk Modifications)**:
   - **Win32 элементы**: колоризация стилей в кэше `uxtheme.dll` и системных цветов (`SetSysColors`).
   - **Win11 SVG иконки**: перехват вызовов `CreateFileW` в `explorer.exe` через легковесную библиотеку `AccentColorizerHook.dll` (на базе MinHook).
   - Системные файлы в `C:\Windows\SystemApps` **никогда не изменяются на диске**.
   - Права `NT SERVICE\TrustedInstaller` остаются нетронутыми.
   - Проверки `DISM /Online /Cleanup-Image /RestoreHealth` и `sfc /scannow` проходят со 100% целостностью.
   - Для работы **не требуются права Администратора (UAC)**.

2. **Защита от циклического падения проводника (Crash Guard)**:
   - Внедрение защищено трехуровневым барьером:
     1. **SEH / Panic Barrier**: любая ошибка внутри хука глушится и мгновенно возвращает оригинальный дескриптор файла.
     2. **Watchdog Canary**: при сбое проводника быстрее 20 секунд более 2 раз подряд инжектор автоматически отключается, оставляя проводник стандартным и стабильным.
     3. **Auto Reset**: при стабильной работе в течение 30 секунд счетчик сбоев автоматически сбрасывается в 0.

3. **Отслеживание смены темы и акцента в реальном времени**:
   - Перехват событий `WM_THEMECHANGED`, `WM_SETTINGCHANGE` (переключение Темная/Светлая тема), `WM_DWMCOLORIZATIONCOLORCHANGED` и `WM_DPICHANGED`.
   - Таймер-дебаунсер устраняет множественные ложные срабатывания Windows.

4. **Полная бесшумность и отсутствие окон**:
   - Скомпилировано с `#![windows_subsystem = "windows"]`.
   - Черные окна консоли не появляются при запуске программы, демона или инъекции.
   - При явном вызове в открытом терминале вывод перенаправляется через `AttachConsole`.

---

## 🚀 Команды

```powershell
# Запуск постоянного фонового процесса (демон без окон)
AccentColorizer.exe

# Фоновый режим с выводом в консоль
AccentColorizer.exe daemon --foreground

# Немедленное применение (память Win32 + внедрение хука в explorer.exe)
AccentColorizer.exe apply --all

# Проверить статус инъекции и состояние Crash Guard
AccentColorizer.exe status

# Сбросить состояние Crash Guard вручную
AccentColorizer.exe reset-guard

# Применить произвольный кастомный цвет
AccentColorizer.exe apply --color #FF5500

# (Опционально) Создать config.toml рядом с exe для тонкой настройки
AccentColorizer.exe --init-config
```

---

## 🛠️ Сборка

```powershell
cargo build --release
```

Бинарные файлы в папке `target/release/`:
- `AccentColorizer.exe` (~605 КБ) — основной демон и инжектор.
- `AccentColorizerHook.dll` (~264 КБ) — модуль перехвата для `explorer.exe`.
