from dataclasses import dataclass
from typing import TYPE_CHECKING, Callable
from backend.apps.models import Application, Applications
import subprocess
from database.service import update_datetime 
from backend.apps.application_search import apps_obj
import asyncio

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


def configure_badges(applications) -> dict[str, list[Application]]:
    """Настройка плашек Applications.

    Функция получает актуальный список из backend и должна вернуть словарь,
    где каждое значение состоит из настоящих объектов Application.
    """
    return apps_obj.filter_apps_by_categories()

def button1_action(window: "MainWindow"):
    """Показывает переданный из backend список приложений."""
    apps = apps_obj.get_apps()
    window.set_applications(apps)
    window.show_applications()


def applications_input_action(
    window: "MainWindow",
    user_input: str,
):
    """Передаёт введённый текст backend и показывает найденные приложения.

    ``backend`` должен принимать строку из поля ввода и возвращать объект
    ``backend.apps.models.Applications``. Сам поиск остаётся на стороне backend.
    """
    applications = apps_obj.search_by_query(user_input)
    if not isinstance(applications, Applications):
        raise TypeError(
            "Backend поиска приложений должен возвращать Applications"
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
    """Заглушка для клика по приложению из сетки Button 1."""
    # Доступные поля: application.name, application.path,
    # application.icon_path.
    asyncio.run(update_datetime(application.name))
    subprocess.run(['gio', 'launch', application.path])
    pass


# Чтобы изменить или добавить кнопку, редактируй только этот список и нужную
# функцию-обработчик выше.
BUTTONS = (
    ButtonConfig("button1", "applications", "Applications", button1_action),
    ButtonConfig("button2", "folder", "Files", button2_action),
    ButtonConfig("button3", "layers", "Actions", button3_action),
    ButtonConfig("button4", "documents", "Clipboard", button4_action),
)
