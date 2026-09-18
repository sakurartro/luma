import asyncio
import signal
import sys
from pathlib import Path

from PySide6 import QtCore, QtWidgets

# Поддерживает запуск как `python frontend/app.py`.
PROJECT_ROOT = Path(__file__).resolve().parent.parent
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from backend.apps.models import Application
from database.init import init_db
from frontend.window_backend import configure_layer_window, prepare_layer_shell


def main(applications: list[Application] | None = None, badges=None):
    asyncio.run(init_db())
    """
    импорт только сейчас так как при импорте start_watcher 
    мы создаем объект apps_obj который в __init__ обращается к БД 
    которая до момента init_db() может быть не создана
    """
    from backend.apps.apps_tracker.tracker import start_watcher
    from backend.apps.application_search import apps_obj
    from frontend.buttons import recent_apps
    from frontend.window import MainWindow

    apps_obj.refresh()
    if not apps_obj._apps:
        apps_obj.initial_scan()
        apps_obj.refresh()
    layer_shell = prepare_layer_shell()
    app = QtWidgets.QApplication(sys.argv)
    app.setQuitOnLastWindowClosed(False)
    app.setApplicationName("luma")
    # На Wayland это значение становится app-id, по которому Niri применяет
    # правило `match app-id="^luma$"` из config.kdl.
    app.setDesktopFileName("luma")

    start_watcher()

    # Плашки по умолчанию строятся из того же списка приложений.
    if applications is None:
        applications = recent_apps(apps_obj._apps)

    window = MainWindow(
        applications=applications,
        badges=badges,
    )
    if layer_shell and app.platformName() == "wayland":
        configure_layer_window(window)
    if "--background" not in sys.argv:
        window.show()

    toggle_requested = False

    def request_toggle(_signum, _frame):
        nonlocal toggle_requested
        toggle_requested = True

    def process_toggle():
        nonlocal toggle_requested
        if toggle_requested:
            toggle_requested = False
            window.toggle_visibility()

    signal.signal(signal.SIGUSR1, request_toggle)
    toggle_timer = QtCore.QTimer()
    toggle_timer.timeout.connect(process_toggle)
    toggle_timer.start(25)

    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
