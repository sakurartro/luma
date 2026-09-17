import asyncio
import subprocess
from dataclasses import dataclass
from datetime import datetime
from typing import TYPE_CHECKING, Callable

from rapidfuzz import fuzz

from backend.apps.application_search import apps_obj
from backend.apps.models import Application
from database.service import update_datetime

if TYPE_CHECKING:
    from frontend.window import MainWindow


ButtonAction = Callable[["MainWindow"], None]

@dataclass(frozen=True)
class ButtonConfig:
    """Настройки одной кнопки."""

    name: str
    icon: str
    tooltip: str
    action: ButtonAction
    size: tuple[int, int] = (42, 42)


def configure_badges(applications: list[Application]) -> dict[str, list[Application]]:
    """Группирует актуальные объекты Application по их категориям."""
    badges: dict[str, list[Application]] = {}
    for application in applications:
        for category in application.categories or []:
            category = category.strip()
            if category:
                badges.setdefault(category, []).append(application)
    return badges


def recent_apps(applications: list[Application]) -> list[Application]:
    return sorted(applications, key=lambda app: app.last_used or datetime.min, reverse=True)


def button1_action(window: "MainWindow"):
    """Показывает актуальные приложения из ApplicationData."""
    apps_obj.refresh()
    window.set_applications(recent_apps(apps_obj._apps))
    window.show_applications()


def applications_input_action(
    window: "MainWindow",
    user_input: str,
) -> list[Application]:
    """Ищет по текущему списку ApplicationData, не создавая старый контейнер."""
    query = user_input.strip().casefold()
    applications = recent_apps(
        [
            application for application in apps_obj._apps
            if not query or query in application.name.casefold()
            or fuzz.partial_ratio(query, application.name.casefold()) >= 80
        ]
    )

    window.set_applications(applications)
    window.show_applications()
    return applications


def button2_action(window: "MainWindow"):
    """Заглушка backend для кнопки файлов."""
    pass


def button3_action(window: "MainWindow"):
    """Заглушка backend для кнопки слоёв/действий."""
    pass


def button4_action(window: "MainWindow"):
    """Заглушка backend для кнопки буфера/документов."""
    pass


def application_action(window: "MainWindow", application: Application):
    """Запускает выбранный desktop-файл и обновляет время использования."""
    asyncio.run(update_datetime(application.app_path))
    subprocess.Popen(['gio', 'launch', application.app_path])
    apps_obj.refresh()


# Чтобы изменить или добавить кнопку, редактируй только этот список и нужную
# функцию-обработчик выше.
BUTTONS = (
    ButtonConfig("button1", "applications", "Applications", button1_action),
    ButtonConfig("button2", "folder", "Files", button2_action),
    ButtonConfig("button3", "layers", "Actions", button3_action),
    ButtonConfig("button4", "documents", "Clipboard", button4_action),
)
