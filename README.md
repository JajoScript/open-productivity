# Open Productivity

Aplicación de escritorio de productividad, de código abierto, para organizar tareas, calendario, objetivos, metas, etc.

Surge como alternativa a las apps de productividad que recolectan datos sin que se sepa bien qué hacen con ellos. Aquí **toda la información se queda en tu dispositivo**: no hay cuentas, ni nube, ni sincronización.


## Tecnologías

- [Rust](https://www.rust-lang.org/) (edición 2024)
- [Slint](https://slint.dev/) para la interfaz

El workspace tiene dos crates:

- `crates/core`: lógica y almacenamiento, sin dependencia de la interfaz.
- `crates/app`: la aplicación de escritorio (binario `open-productivity`).


## Licencia

Copyright (C) 2026 JajoScript

Este proyecto es software libre bajo la [GNU General Public License v3.0 o posterior](LICENSE) (`GPL-3.0-or-later`): puedes usarlo, estudiarlo, modificarlo y redistribuirlo, siempre que las versiones que distribuyas sigan siendo GPLv3. Se distribuye sin ninguna garantía.

Slint se usa bajo su licencia GPLv3.
