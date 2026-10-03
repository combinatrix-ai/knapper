# PKM fixture

[[Tasks/Todo]] · [[Books/Field Guide]] · [[Books/Notebook]]

Queries and expected results are in `cases.json`. Run against a disposable copy.

## reading-list

```dataview
TABLE WITHOUT ID file.name, rating, status FROM "Books" SORT rating DESC
```

## todo-status-and-priority

```dataview
TABLE WITHOUT ID t.text, t.status, t.completed, t.p FROM "Tasks" FLATTEN file.tasks AS t SORT t.line
```

## todo-due-date

```dataview
TABLE WITHOUT ID t.p, dateformat(t.due, "yyyy-MM-dd") FROM "Tasks" FLATTEN file.tasks AS t WHERE t.due
```

## daily-aggregate

```dataview
TABLE WITHOUT ID length(rows), sum(rows.messages) FROM "Daily" GROUP BY true
```

## author-link

```dataview
TABLE WITHOUT ID file.name, meta(author).path FROM "Books" SORT file.name
```

## bookmark-membership

```dataview
LIST WITHOUT ID file.path WHERE file.starred SORT file.path
```

## open-tasks

```dataview
TASK FROM "Tasks" WHERE !completed SORT line
```
