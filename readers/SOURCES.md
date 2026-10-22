# Graded reader source documents

Input for [`prepare-readers`](../docs/src/reference/cli-reference.md#prepare-readers)
and [`check-reader-pdf`](../docs/src/reference/cli-reference.md#check-reader-pdf).
Each book sits at `readers/<series>/<level>/<title>.pdf`, and the `File` cell
is its path relative to this directory.

The `Translate` column is read by `prepare-readers`, not merely by people.
Importing a book sends its text to the configured model and serves a
translation of it to learners; for a commercial reader both are
reproductions, so a book is extracted and translated only when its row here
says `yes`. A book with no row, or with any other value, is never read, and
there is no flag that overrides the cell.

The `Level` column is the publisher's level. For Penguin Readers it maps to
a CEFR level (0 and 1 to A1, 2 to A2, 3 and 4 to B1, 5 and 6 to B2); a value
in the `CEFR` column replaces the mapped one, and an unlevelled title must
state one before it can be cleared. The `Pages` column, as `6-33` or `6–33`,
names the body of the book when the importer cannot find it by its
`Chapter 1` heading; changing it makes a new revision of the book, and the
learner's place is kept.

These 85 rows are the Penguin titles whose PDFs carry a text layer. The
scanned titles on the example shelf, and the answer keys, factsheets, tests,
worksheets and teacher's notes filed beside four of the books, are not
listed.

| Series | Level | Title | File | CEFR | Pages | Translate | Notes |
|--------|-------|-------|------|------|-------|-----------|-------|
| Penguin Readers | 0-1 | Brown Eyes | `Penguin Readers/Level 0-1/Brown Eyes.pdf` |  |  |  |  |
| Penguin Readers | 0-1 | Carnival | `Penguin Readers/Level 0-1/Carnival.pdf` |  |  |  |  |
| Penguin Readers | 0-1 | Girl Meets Boy | `Penguin Readers/Level 0-1/Girl Meets Boy.pdf` |  |  |  |  |
| Penguin Readers | 0-1 | Marcel and the White Star | `Penguin Readers/Level 0-1/Marcel and the White Star.pdf` |  |  |  |  |
| Penguin Readers | 0-1 | Newspaper Chase | `Penguin Readers/Level 0-1/Newspaper Chase/Newspaper Chase.pdf` |  |  |  |  |
| Penguin Readers | 0-1 | Prince William | `Penguin Readers/Level 0-1/Prince William.pdf` |  |  |  |  |
| Penguin Readers | 0-1 | The Adventures of Tom Sawyer | `Penguin Readers/Level 0-1/The Adventures of Tom Sawyer.pdf` |  |  |  |  |
| Penguin Readers | 0-1 | The Last Photo | `Penguin Readers/Level 0-1/The Last Photo.pdf` |  |  |  |  |
| Penguin Readers | 2 | Anne of Green Gables | `Penguin Readers/Level 2/Anne of Green Gables.pdf` |  |  |  |  |
| Penguin Readers | 2 | Black Beauty | `Penguin Readers/Level 2/Black Beauty.pdf` |  |  |  |  |
| Penguin Readers | 2 | London | `Penguin Readers/Level 2/London.pdf` |  |  |  |  |
| Penguin Readers | 2 | Lost Love and Other Stories | `Penguin Readers/Level 2/Lost Love and Other Stories/Lost Love and Other Stories.pdf` |  |  |  |  |
| Penguin Readers | 2 | Moby Dick | `Penguin Readers/Level 2/Moby Dick.pdf` |  |  |  |  |
| Penguin Readers | 2 | Mr. Bean in Town | `Penguin Readers/Level 2/Mr. Bean in Town.pdf` |  |  |  |  |
| Penguin Readers | 2 | Robin Hood | `Penguin Readers/Level 2/Robin Hood.pdf` |  |  |  |  |
| Penguin Readers | 2 | Round the World in 80 Days | `Penguin Readers/Level 2/Round the World in 80 Days.pdf` |  |  |  |  |
| Penguin Readers | 2 | Simply Suspense | `Penguin Readers/Level 2/Simply Suspense.pdf` |  |  |  |  |
| Penguin Readers | 2 | Stranger than Fiction Urban Myths | `Penguin Readers/Level 2/Stranger than Fiction Urban Myths.pdf` |  |  |  |  |
| Penguin Readers | 2 | The Lady in the Lake | `Penguin Readers/Level 2/The Lady in the Lake.pdf` |  |  |  |  |
| Penguin Readers | 2 | The Mummy Returns | `Penguin Readers/Level 2/The Mummy Returns.pdf` |  |  |  |  |
| Penguin Readers | 2 | The Wave | `Penguin Readers/Level 2/The Wave.pdf` |  |  |  |  |
| Penguin Readers | 2 | Treasure Island | `Penguin Readers/Level 2/Treasure Island.pdf` |  |  |  |  |
| Penguin Readers | 3 | A History of Britain | `Penguin Readers/Level 3/A History of Britain.pdf` |  |  |  |  |
| Penguin Readers | 3 | Dr Jekyll and Mr Hyde | `Penguin Readers/Level 3/Dr Jekyll and Mr Hyde.pdf` |  |  |  |  |
| Penguin Readers | 3 | Forrest Gump | `Penguin Readers/Level 3/Forrest Gump.pdf` |  |  |  |  |
| Penguin Readers | 3 | Hamlet | `Penguin Readers/Level 3/Hamlet.pdf` |  |  |  |  |
| Penguin Readers | 3 | How to be an Alien | `Penguin Readers/Level 3/How to be an Alien.pdf` |  |  |  |  |
| Penguin Readers | 3 | Manchester United | `Penguin Readers/Level 3/Manchester United.pdf` |  |  |  |  |
| Penguin Readers | 3 | More Heroic Failures | `Penguin Readers/Level 3/More Heroic Failures.pdf` |  |  |  |  |
| Penguin Readers | 3 | New York | `Penguin Readers/Level 3/New York.pdf` |  |  |  |  |
| Penguin Readers | 3 | Sense and Sensibility | `Penguin Readers/Level 3/Sense and Sensibility.pdf` |  |  |  |  |
| Penguin Readers | 3 | Stargate | `Penguin Readers/Level 3/Stargate.pdf` |  |  |  |  |
| Penguin Readers | 3 | The Ghosts of Izieu | `Penguin Readers/Level 3/The Ghosts of Izieu.pdf` |  |  |  |  |
| Penguin Readers | 3 | The Horse Whisperer | `Penguin Readers/Level 3/The Horse Whisperer.pdf` |  |  |  |  |
| Penguin Readers | 3 | The Red Badge of Courage | `Penguin Readers/Level 3/The Red Badge of Courage.pdf` |  |  |  |  |
| Penguin Readers | 3 | The Turn of the Screw | `Penguin Readers/Level 3/The Turn of the Screw.pdf` |  |  |  |  |
| Penguin Readers | 3 | The Young King and Other Stories | `Penguin Readers/Level 3/The Young King and Other Stories.pdf` |  |  |  |  |
| Penguin Readers | 3 | Titanic | `Penguin Readers/Level 3/Titanic/Titanic.pdf` |  |  |  |  |
| Penguin Readers | 4 | 1984 | `Penguin Readers/Level 4/1984.pdf` |  |  |  |  |
| Penguin Readers | 4 | About a Boy | `Penguin Readers/Level 4/About a Boy.pdf` |  |  |  |  |
| Penguin Readers | 4 | As Time Goes By | `Penguin Readers/Level 4/As Time Goes By.pdf` |  |  |  |  |
| Penguin Readers | 4 | Cinderella Man | `Penguin Readers/Level 4/Cinderella Man.pdf` |  |  |  |  |
| Penguin Readers | 4 | Crime Story Collection | `Penguin Readers/Level 4/Crime Story Collection/Crime Story Collection (2).pdf` |  |  |  |  |
| Penguin Readers | 4 | Gladiator | `Penguin Readers/Level 4/Gladiator.pdf` |  |  |  |  |
| Penguin Readers | 4 | Lorna Doone | `Penguin Readers/Level 4/Lorna Doone.pdf` |  |  |  |  |
| Penguin Readers | 4 | Management Gurus | `Penguin Readers/Level 4/Management Gurus.pdf` |  |  |  |  |
| Penguin Readers | 4 | Primary Colors | `Penguin Readers/Level 4/Primary Colors.pdf` |  |  |  |  |
| Penguin Readers | 4 | Shakespeare His Life and Plays | `Penguin Readers/Level 4/Shakespeare His Life and Plays.pdf` |  |  |  |  |
| Penguin Readers | 4 | Strangers on a Train | `Penguin Readers/Level 4/Strangers on a Train.pdf` |  |  |  |  |
| Penguin Readers | 4 | The Dream and other stories | `Penguin Readers/Level 4/The Dream and other stories.pdf` |  |  |  |  |
| Penguin Readers | 4 | The Full Monty | `Penguin Readers/Level 4/The Full Monty.pdf` |  |  |  |  |
| Penguin Readers | 4 | The Godfather | `Penguin Readers/Level 4/The Godfather.pdf` |  |  |  |  |
| Penguin Readers | 4 | The House of Stairs | `Penguin Readers/Level 4/The House of Stairs.pdf` |  |  |  |  |
| Penguin Readers | 4 | The Locked Room | `Penguin Readers/Level 4/The Locked Room.pdf` |  |  |  |  |
| Penguin Readers | 4 | The Time Machine | `Penguin Readers/Level 4/The Time Machine.pdf` |  |  |  |  |
| Penguin Readers | 4 | Three Great Plays of Shakespeare | `Penguin Readers/Level 4/Three Great Plays of Shakespeare.pdf` |  |  |  |  |
| Penguin Readers | 4 | Women in Business | `Penguin Readers/Level 4/Women in Business.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Airport | `Penguin Readers/Level 5-6/Airport.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | A Time to Kill | `Penguin Readers/Level 5-6/A Time to Kill.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Benetton | `Penguin Readers/Level 5-6/Benetton.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Business at the Speed of Thought | `Penguin Readers/Level 5-6/Business at the Speed of Thought.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Four Weddings and a Funeral | `Penguin Readers/Level 5-6/Four Weddings and a Funeral.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Jude the Obscure | `Penguin Readers/Level 5-6/Jude the Obscure.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Misery | `Penguin Readers/Level 5-6/Misery.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | More Tales from Shakespeare | `Penguin Readers/Level 5-6/More Tales from Shakespeare.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | On the Road | `Penguin Readers/Level 5-6/On the Road.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Pride and Prejudice | `Penguin Readers/Level 5-6/Pride and Prejudice.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Prime Suspect | `Penguin Readers/Level 5-6/Prime Suspect.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Tales of Mystery and Imagination | `Penguin Readers/Level 5-6/Tales of Mystery and Imagination.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | Taste and Other Tales | `Penguin Readers/Level 5-6/Taste and Other Tales.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | The Body | `Penguin Readers/Level 5-6/The Body.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | The Firm | `Penguin Readers/Level 5-6/The Firm.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | The Grass is Singing | `Penguin Readers/Level 5-6/The Grass is Singing.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | The Invisible Man | `Penguin Readers/Level 5-6/The Invisible Man.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | The Pelican Brief | `Penguin Readers/Level 5-6/The Pelican Brief.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | The Story of the Internet | `Penguin Readers/Level 5-6/The Story of the Internet.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | The War of the Worlds | `Penguin Readers/Level 5-6/The War of the Worlds.pdf` |  |  |  |  |
| Penguin Readers | 5-6 | World Folktales | `Penguin Readers/Level 5-6/World Folktales.pdf` |  |  |  |  |
| Penguin Readers | Unlevelled | Another World | `Penguin Readers/Unlevelled/Another World.pdf` |  |  |  | No publisher level; set CEFR before clearing |
| Penguin Readers | Unlevelled | Dante's Peak | `Penguin Readers/Unlevelled/Dante's Peak.pdf` |  |  |  | No publisher level; set CEFR before clearing |
| Penguin Readers | Unlevelled | Mysteries of the Unexplained | `Penguin Readers/Unlevelled/Mysteries of the Unexplained.pdf` |  |  |  | No publisher level; set CEFR before clearing |
| Penguin Readers | Unlevelled | The Count of Monte Cristo | `Penguin Readers/Unlevelled/The Count of Monte Cristo (Audio added).pdf` |  |  |  | No publisher level; set CEFR before clearing |
| Penguin Readers | Unlevelled | The Earthquake | `Penguin Readers/Unlevelled/The Earthquake.pdf` |  |  |  | No publisher level; set CEFR before clearing |
| Penguin Readers | Unlevelled | The Incredible Journey | `Penguin Readers/Unlevelled/The Incredible Journey.pdf` |  |  |  | No publisher level; set CEFR before clearing |
| Penguin Readers | Unlevelled | The Nightmare Before Christmas | `Penguin Readers/Unlevelled/The Nightmare Before Christmas.pdf` |  |  |  | No publisher level; set CEFR before clearing |
