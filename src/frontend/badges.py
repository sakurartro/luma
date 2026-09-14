from collections.abc import Iterable, Mapping

from PySide6 import QtCore, QtWidgets

from backend.apps.models import Badges


def badges_from_dict(badges: Mapping[str, object]) -> list[Badges]:
    """Преобразует словарь в backend-модели плашек.

    Поддерживает как одну плашку в формате модели ``Badges``, так и набор
    групп вида ``{название: [Application, ...]}``.
    """
    if "name" in badges and "apps" in badges:
        return [Badges(**dict(badges))]

    models = []
    for name, value in badges.items():
        if isinstance(value, Badges):
            models.append(value)
        elif isinstance(value, Mapping) and "apps" in value:
            data = dict(value)
            data.setdefault("name", str(name))
            models.append(Badges(**data))
        else:
            models.append(Badges(name=str(name), apps=list(value or [])))
    return models


class BadgeButton(QtWidgets.QPushButton):
    """Компактная плашка для одной группы приложений."""

    selected = QtCore.Signal(object)

    def __init__(self, badge: Badges, parent=None):
        super().__init__(badge.name, parent)
        self.badge = badge
        self.setObjectName("badgeButton")
        self.setProperty("active", False)
        self.setFixedHeight(28)
        self.setCursor(QtCore.Qt.PointingHandCursor)
        self.setToolTip(f"{badge.name}: {len(badge.apps)}")
        self.clicked.connect(lambda: self.selected.emit(self.badge))

    def set_active(self, active: bool):
        self.setProperty("active", active)
        self.style().unpolish(self)
        self.style().polish(self)


class BadgesBar(QtWidgets.QWidget):
    """Горизонтальный ряд плашек, созданных из моделей ``Badges``."""

    badge_selected = QtCore.Signal(object)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("badgesBar")

        self.row = QtWidgets.QHBoxLayout(self)
        self.row.setContentsMargins(0, 0, 0, 0)
        self.row.setSpacing(6)
        self.row.setAlignment(QtCore.Qt.AlignLeft)

        self.badges: list[Badges] = []
        self.buttons: list[BadgeButton] = []
        self.hide()

    def set_badges(
        self,
        badges: Mapping[str, object] | Iterable[Badges] | None,
    ) -> list[Badges]:
        """Устанавливает плашки из словаря или готовых моделей ``Badges``."""
        while self.row.count():
            item = self.row.takeAt(0)
            if item.widget() is not None:
                item.widget().hide()
                item.widget().deleteLater()

        if badges is None:
            models = []
        elif isinstance(badges, Mapping):
            models = badges_from_dict(badges)
        else:
            models = [
                badge
                if isinstance(badge, Badges)
                else Badges.model_validate(badge)
                for badge in badges
            ]

        self.badges = models
        self.buttons = []
        for badge in models:
            button = BadgeButton(badge)
            button.selected.connect(self._select_badge)
            self.row.addWidget(button)
            self.buttons.append(button)

        self.row.addStretch(1)
        self.setVisible(bool(models))
        return models

    @QtCore.Slot(object)
    def _select_badge(self, selected: Badges):
        for button in self.buttons:
            button.set_active(button.badge is selected)
        self.badge_selected.emit(selected)

    def clear_selection(self):
        for button in self.buttons:
            button.set_active(False)
