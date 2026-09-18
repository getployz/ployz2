import * as React from "react"
import { CheckIcon, XIcon } from "lucide-react"
import { Combobox as ComboboxPrimitive } from "@base-ui/react/combobox"
import { Combobox, ComboboxContent, ComboboxEmpty, ComboboxItem, ComboboxList, ComboboxTrigger } from "#/components/ui/combobox"

import { cn } from "#/lib/utils"
import { FieldError } from "#/components/ui/field"
import { Spinner } from "#/components/ui/spinner"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
  InputGroupText,
  InputGroupTextarea,
} from "#/components/ui/input-group"

function ConfirmableInput({
  value,
  onValueChange,
  onConfirm,
  onCancel,
  isDirty = false,
  isPending = false,
  isChanged = false,
  suffix,
  error,
  className,
  multiline = false,
  suggestions,
  suggestionsLoading = false,
  suggestionsMessage,
  onSuggestionSelect,
  ...props
}: Omit<
  React.ComponentProps<"input"> & React.ComponentProps<"textarea">,
  "defaultValue" | "onChange" | "onSubmit" | "value"
> & {
  value: string
  onValueChange: (value: string) => void
  onConfirm?: () => void
  onCancel?: () => void
  isDirty?: boolean
  isPending?: boolean
  isChanged?: boolean
  suffix?: React.ReactNode
  error?: React.ReactNode
  multiline?: boolean
  suggestions?: string[]
  suggestionsLoading?: boolean
  suggestionsMessage?: string
  onSuggestionSelect?: (value: string) => void
}) {
  const [suggestionsOpen, setSuggestionsOpen] = React.useState(false)
  const [highlightedSuggestion, setHighlightedSuggestion] = React.useState<string | undefined>(undefined)
  // SAFETY: input and textarea onKeyDown handlers are the same function; their event element types don't unify.
  const onKeyDown = props.onKeyDown as
    | ((event: React.KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => void)
    | undefined

  function updateDraftValue(
    e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>,
  ) {
    onValueChange(e.target.value)
  }

  function handleConfirm() {
    if (!isDirty) {
      return
    }

    onConfirm?.()
  }

  function handleCancel() {
    if (!isDirty) {
      return
    }

    onCancel?.()
  }

  function handleKeyDown(
    e: React.KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>,
  ) {
    if (suggestionsOpen && e.key === "Escape") return
    if (suggestionsOpen && e.key === "Enter" && highlightedSuggestion !== undefined) return
    if (e.key === "Enter" && isDirty) {
      if (multiline && !e.metaKey && !e.ctrlKey) {
        onKeyDown?.(e)
        return
      }
      e.preventDefault()
      setSuggestionsOpen(false)
      handleConfirm()
      return
    }
    if (e.key === "Escape" && isDirty) {
      e.preventDefault()
      e.stopPropagation()
      e.nativeEvent.stopImmediatePropagation()
      handleCancel()
      return
    }
    onKeyDown?.(e)
  }

  const field = (
    <div className={cn("flex flex-col gap-1", className)}>
      <InputGroup data-changed={isChanged || undefined}>
        {multiline ? (
          <InputGroupTextarea
            {...props}
            value={value}
            aria-invalid={props["aria-invalid"] ?? !!error}
            disabled={props.disabled}
            onChange={updateDraftValue}
            onKeyDown={handleKeyDown}
          />
        ) : suggestions ? (
          <ComboboxPrimitive.Input
            {...props}
            render={<InputGroupInput />}
            aria-invalid={props["aria-invalid"] ?? !!error}
            onFocus={(event) => {
              setSuggestionsOpen(true)
              props.onFocus?.(event)
            }}
            onKeyDown={handleKeyDown}
          />
        ) : (
          <InputGroupInput
            {...props}
            value={value}
            aria-invalid={props["aria-invalid"] ?? !!error}
            disabled={props.disabled}
            onChange={updateDraftValue}
            onKeyDown={handleKeyDown}
          />
        )}
        {suffix || (isDirty && !isPending) || suggestions ? (
          <InputGroupAddon align="inline-end">
            {suffix ? <InputGroupText>{suffix}</InputGroupText> : null}
            {suggestions ? (
              <InputGroupButton size="icon-xs" variant="ghost" render={<ComboboxTrigger />} aria-label="Show suggestions" />
            ) : null}
            {isDirty && !isPending ? (
              <>
                <InputGroupButton
                  size="icon-xs"
                  variant="ghost"
                  onClick={handleCancel}
                  aria-label="Cancel"
                >
                  <XIcon />
                </InputGroupButton>
                <InputGroupButton
                  size="icon-xs"
                  variant="ghost"
                  onClick={handleConfirm}
                  aria-label="Confirm"
                >
                  <CheckIcon />
                </InputGroupButton>
              </>
            ) : null}
          </InputGroupAddon>
        ) : null}
        {isPending ? (
          <InputGroupAddon align="inline-end">
            <Spinner />
          </InputGroupAddon>
        ) : null}
      </InputGroup>
      {error ? <FieldError>{error}</FieldError> : null}
    </div>
  )

  if (!suggestions) return field

  return (
    <Combobox
      items={suggestions}
      value={value || null}
      inputValue={value}
      onInputValueChange={(next, details) => {
        if (details.reason !== "item-press" || !onSuggestionSelect) onValueChange(next)
      }}
      onValueChange={(next) => {
        if (next !== null) (onSuggestionSelect ?? onValueChange)(next)
      }}
      onItemHighlighted={setHighlightedSuggestion}
      open={suggestionsOpen}
      onOpenChange={(open) => {
        setSuggestionsOpen(open)
        if (!open) setHighlightedSuggestion(undefined)
      }}
      openOnInputClick
      disabled={props.disabled}
    >
      {field}
      <ComboboxContent>
        <ComboboxEmpty>
          {suggestionsLoading ? <Spinner aria-label="Loading suggestions" /> : suggestionsMessage ?? "No matches. Enter a custom path."}
        </ComboboxEmpty>
        <ComboboxList>
          {(item: string) => <ComboboxItem key={item} value={item}>{item}</ComboboxItem>}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  )
}

export { ConfirmableInput }
