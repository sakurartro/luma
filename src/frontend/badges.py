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
        self.setFixedSize(108, 28)
        self.setCursor(QtCore.Qt.PointingHandCursor)
        self.setToolTip(f"{badge.name}: {len(badge.apps)}")
        self.clicked.connect(lambda: self.selected.emit(self.badge))

    def set_active(self, active: bool):
        self.setProperty("active", active)
        self.style().unpolish(self)
        self.style().polish(self)


class BadgesBar(QtWidgets.QScrollArea):
    """Горизонтальный ряд плашек, созданных из моделей ``Badges``."""

    badge_selected = QtCore.Signal(object)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("badgesBar")
        self.setFixedHeight(36)
        self.setWidgetResizable(False)
        self.setHorizontalScrollBarPolicy(QtCore.Qt.ScrollBarAlwaysOff)
        self.setVerticalScrollBarPolicy(QtCore.Qt.ScrollBarAlwaysOff)
        self.setFrameShape(QtWidgets.QFrame.NoFrame)

        self.content = QtWidgets.QWidget()
        self.row = QtWidgets.QHBoxLayout(self.content)
        self.row.setContentsMargins(0, 4, 0, 4)
        self.row.setSpacing(6)
        self.row.setAlignment(QtCore.Qt.AlignLeft)
        self.row.setSizeConstraint(QtWidgets.QLayout.SetFixedSize)
        self.setWidget(self.content)
        self.content.installEventFilter(self)
        self.viewport().installEventFilter(self)

        self.badges: list[Badges] = []
        self.buttons: list[BadgeButton] = []
        self._drag_source = None
        self._dragged = False
        self.hide()

    def set_badges(
        self,
        badges: Mapping[str, object] | Iterable[Badges] | None,
    ) -> list[Badges]:
        """Устанавливает плашки из словаря или готовых моделей ``Badges``."""
        reusable_buttons: dict[str, list[BadgeButton]] = {}
        for button in self.buttons:
            reusable_buttons.setdefault(button.badge.name, []).append(button)
        while self.row.count():
            item = self.row.takeAt(0)
            if item.widget() is not None:
                item.widget().hide()

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
            matching_buttons = reusable_buttons.get(badge.name, [])
            button = matching_buttons.pop() if matching_buttons else None
            if button is None:
                button = BadgeButton(badge)
                button.selected.connect(self._select_badge)
                button.installEventFilter(self)
            else:
                button.badge = badge
                button.setToolTip(f"{badge.name}: {len(badge.apps)}")
                if button.property("active"):
                    button.set_active(False)
            self.row.addWidget(button)
            button.show()
            self.buttons.append(button)

        for buttons in reusable_buttons.values():
            for button in buttons:
                button.deleteLater()

        self.content.adjustSize()
        self.horizontalScrollBar().setValue(0)
        self.setVisible(bool(models))
        return models

    def wheelEvent(self, event):
        pixels = event.pixelDelta()
        angles = event.angleDelta()
        delta = pixels.x() or pixels.y() or angles.x() // 2 or angles.y() // 2
        bar = self.horizontalScrollBar()
        bar.setValue(bar.value() - delta)
        event.accept()

    def eventFilter(self, watched, event):
        if event.type() == QtCore.QEvent.MouseButtonPress and event.button() == QtCore.Qt.LeftButton:
            self._drag_source = watched
            self._drag_start_x = event.globalPosition().x()
            self._drag_start_scroll = self.horizontalScrollBar().value()
            self._dragged = False
        elif event.type() == QtCore.QEvent.MouseMove and self._drag_source is watched and event.buttons() & QtCore.Qt.LeftButton:
            distance = event.globalPosition().x() - self._drag_start_x
            if abs(distance) >= 6:
                self._dragged = True
            if self._dragged:
                self.horizontalScrollBar().setValue(self._drag_start_scroll - int(distance))
                if isinstance(watched, BadgeButton):
                    watched.setDown(False)
                return True
        elif event.type() == QtCore.QEvent.MouseButtonRelease and event.button() == QtCore.Qt.LeftButton and self._drag_source is watched:
            self._drag_source = None
            if self._dragged:
                if isinstance(watched, BadgeButton):
                    watched.setDown(False)
                return True
        return super().eventFilter(watched, event)

    @QtCore.Slot(object)
    def _select_badge(self, selected: Badges):
        for button in self.buttons:
            button.set_active(button.badge is selected)
        self.badge_selected.emit(selected)

    def clear_selection(self):
        for button in self.buttons:
            button.set_active(False)
