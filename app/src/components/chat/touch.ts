import { createContext, useContext } from "react"

/** The chat is on a touchscreen (the phone app): thumb-sized controls, no key hints, and
 *  layouts that stack instead of crowding one row. The desktop leaves it off. */
export const TouchContext = createContext(false)

export const useTouch = () => useContext(TouchContext)
